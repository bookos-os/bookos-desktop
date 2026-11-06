//! El emergente de las notificaciones.
//!
//! La lista viene de fuera: quien atiende `org.freedesktop.Notifications` es el
//! compositor —el shell no habla D-Bus, para poder estar pintado en el primer
//! frame— y la cola vive en [`crate::notificaciones::Registro`]. Aquí solo se
//! guarda una copia de lo que hay que dibujar, que se refresca cuando cambia.
//!
//! «No molestar» sí es estado de esta tarjeta, con su caducidad.

use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::control::{self, GRUPO};
use super::lista::{self, interruptor};
use super::{Ancla, Tecla};

/// Alto de la fila de «No molestar», con su explicación y su interruptor.
const BLOQUE: f32 = 52.0;
/// Alto de una notificación de la lista: dos renglones y su icono.
const FILA_NOTIF: f32 = 52.0;
/// Cuántas caben antes de que la tarjeta sea más alta que la pantalla. Las
/// demás siguen en la cola —y en el contador del panel—, pero no se dibujan.
const VISIBLES: usize = 4;
/// Alto de un renglón del cuerpo desplegado, a 11 px.
const LINEA_DET: f32 = 15.0;
/// Renglones máximos del cuerpo desplegado. Con más, la tarjeta pasaría de la
/// pantalla; el último termina en «…».
const MAX_LINEAS: usize = 8;
const ALTO_ACCION: f32 = 28.0;
/// Alto de la línea de la app y de la del resumen: fijos para que el alto de
/// una fila desplegada se calcule sin medir texto.
const ALTO_CAB: f32 = 14.0;
const ALTO_RES: f32 = 17.0;
const PAD_V: f32 = 6.0;
/// Ancho de los textos de una fila: la baldosa menos su padding, el icono y el
/// hueco entre los dos.
const ANCHO_TEXTO: f32 = lista::BALDOSA - 24.0 - 26.0 - 12.0;
/// Zona de la ✕ de cerrar, pegada al borde derecho de la fila.
const CIERRE: f32 = 30.0;
/// Alto de cada botón de duración.
const DURACION: f32 = 34.0;
/// Hueco entre las piezas del grupo: filas, botones y bloques.
const HUECO: f32 = 6.0;
/// El hueco de la hora —o de la ✕ que la sustituye—: «hace 44 min» a 11 px cabe
/// con holgura, y a ancho fijo la cabecera no se descuadra con el texto.
const HORA_ANCHO: f32 = 64.0;
/// Hueco entre los dos chips de la cabecera.
const ENTRE_CHIPS: f32 = 8.0;
/// La etiqueta del botón que vacía la lista. «Borrar» y no «Borrar todo»: con
/// el título y «Config» no cabía en los 296 px de la cabecera y el título
/// quedaba pegado al botón. Lo que borra está justo debajo, y se lee igual.
/// Una sola constante para el dibujo y para la zona de clic.
const BORRAR: &str = "Borrar";

/// Cuánto dura el silencio, en la rejilla de dos por dos del diseño.
const DURACIONES: &[(&str, Option<Duration>)] = &[
    ("1 hora", Some(Duration::from_secs(3600))),
    ("4 horas", Some(Duration::from_secs(4 * 3600))),
    ("8 horas", Some(Duration::from_secs(8 * 3600))),
    ("Hasta desactivarlo", None),
];

pub struct Notificaciones {
    /// Cuándo se activó el silencio y hasta cuándo. `None` = no está puesto.
    silencio: Option<(Instant, Option<Duration>)>,
    /// Qué duración está elegida, para marcarla.
    duracion: usize,
    señalada: tema::Realce,
    /// El puntero está sobre el interruptor.
    sobre_interruptor: bool,
    /// El recorrido de la bolita, siguiendo al silencio.
    interruptor: tema::Transicion,
    /// Lo que hay que enseñar, copiado de la cola del shell. Copia y no
    /// préstamo porque la tarjeta se dibuja desde `view(&self)` y el `Shell`
    /// que tiene la cola es quien la llama: prestársela sería prestarse a sí
    /// mismo.
    lista: Vec<crate::notificaciones::Notificacion>,
    /// La fila señalada de la lista, para el realce y para la ✕.
    fila: tema::Realce,
    /// Fila activa para navegación sin ratón.
    seleccionada: Option<usize>,
    /// La notificación desplegada, por id: una sola a la vez, como en Android.
    /// Por id y no por posición porque la lista se reordena al llegar avisos.
    desplegada: Option<u32>,
}

impl Notificaciones {
    // 0.1.0
    pub fn poner_silencio(&mut self, silencio: Option<(Instant, Option<Duration>)>) {
        self.silencio = silencio;
        self.interruptor.fijar(self.silenciado() as u8 as f32);
    }

    /// El silencio tal y como estaba, para que abrir y cerrar la tarjeta no lo
    /// olvide: su dueño es el shell, que sigue vivo con la tarjeta cerrada.
    // 0.1.0
    pub fn con_silencio(silencio: Option<(Instant, Option<Duration>)>) -> Self {
        let mut n = Self::new();
        n.silencio = silencio;
        n.interruptor.fijar(n.silenciado() as u8 as f32);
        n
    }

    /// Cómo está el silencio ahora, para que el shell se lo quede.
    // 0.1.0
    pub fn silencio(&self) -> Option<(Instant, Option<Duration>)> {
        self.silencio
    }

    // 0.1.0
    pub fn new() -> Self {
        Self {
            silencio: None,
            // «Hasta desactivarlo» por defecto: es lo que espera quien pulsa el
            // interruptor sin mirar las opciones.
            duracion: 3,
            señalada: tema::Realce::nuevo(),
            sobre_interruptor: false,
            interruptor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            lista: Vec::new(),
            fila: tema::Realce::nuevo(),
            seleccionada: None,
            desplegada: None,
        }
    }

    /// El cuerpo partido en renglones del ancho de la fila.
    // 0.1.0
    fn detalle(notif: &crate::notificaciones::Notificacion) -> Vec<String> {
        crate::escritorio::lineas(&notif.cuerpo, ANCHO_TEXTO, 11.0, MAX_LINEAS)
    }

    /// Lo que se ofrece como botón al desplegar: las acciones de la aplicación
    /// y, si la tiene, «Abrir» por la acción `default`, que la especificación
    /// reserva para pulsar el aviso y que en la lista ya no se dispara al pulsar
    /// la fila —pulsarla ahora despliega—.
    // 0.1.0
    fn botones(notif: &crate::notificaciones::Notificacion) -> Vec<(String, String)> {
        let mut botones: Vec<_> = notif
            .acciones
            .iter()
            .filter(|a| a.clave != "default")
            .map(|a| (a.clave.clone(), a.etiqueta.clone()))
            .collect();
        if notif.acciones.iter().any(|a| a.clave == "default") {
            botones.push(("default".into(), "Abrir".into()));
        }
        botones
    }

    /// ¿Tiene algo más que enseñar al desplegarse? Si no, la fila se comporta
    /// como siempre: un toque la ejecuta o la cierra.
    // 0.1.0
    fn desplegable(notif: &crate::notificaciones::Notificacion) -> bool {
        !Self::botones(notif).is_empty() || Self::detalle(notif).len() > 1
    }

    // 0.1.0
    fn esta_desplegada(&self, notif: &crate::notificaciones::Notificacion) -> bool {
        self.desplegada == Some(notif.id) && Self::desplegable(notif)
    }

    /// Alto de una fila: la de siempre, o la que pide su cuerpo entero y sus
    /// botones si está desplegada.
    // 0.1.0
    fn alto_fila(&self, notif: &crate::notificaciones::Notificacion) -> f32 {
        if !self.esta_desplegada(notif) {
            return FILA_NOTIF;
        }
        let botones = if Self::botones(notif).is_empty() {
            0.0
        } else {
            HUECO + ALTO_ACCION
        };
        PAD_V * 2.0 + ALTO_CAB + ALTO_RES + Self::detalle(notif).len() as f32 * LINEA_DET + botones
    }

    /// Dónde empieza cada fila visible, relativo a la lista, y cuánto mide.
    // 0.1.0
    fn geometria_filas(&self) -> Vec<(f32, f32)> {
        let mut y = 0.0;
        self.lista
            .iter()
            .take(VISIBLES)
            .map(|n| {
                let alto = self.alto_fila(n);
                let fila = (y, alto);
                y += alto + HUECO;
                fila
            })
            .collect()
    }

    /// Qué botón de la fila `i` desplegada cae en un punto.
    #[cfg(test)]
    fn pulsar_sin_efecto(&self, x: f32, y: f32) -> Option<String> {
        self.fila_en(x, y).and_then(|i| self.boton_en(i, x, y))
    }

    // 0.1.0
    fn boton_en(&self, i: usize, x: f32, y: f32) -> Option<String> {
        let notif = self.lista.get(i)?;
        let botones = Self::botones(notif);
        if !self.esta_desplegada(notif) || botones.is_empty() {
            return None;
        }
        let (arriba, _) = self.geometria_filas()[i];
        let y0 = self.y_lista()
            + arriba
            + PAD_V
            + ALTO_CAB
            + ALTO_RES
            + Self::detalle(notif).len() as f32 * LINEA_DET
            + HUECO;
        if !(y0..=y0 + ALTO_ACCION).contains(&y) {
            return None;
        }
        let n = botones.len();
        let ancho = (ANCHO_TEXTO - HUECO * (n as f32 - 1.0)) / n as f32;
        let x0 = lista::MARGEN + GRUPO + 12.0 + 26.0 + 12.0;
        botones.into_iter().enumerate().find_map(|(k, (clave, _))| {
            let ini = x0 + k as f32 * (ancho + HUECO);
            (ini..=ini + ancho).contains(&x).then_some(clave)
        })
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        // Las duraciones solo ocupan sitio cuando el silencio está puesto: es
        // lo que dice el diseño y evita cuatro botones que no hacen nada.
        let duraciones = if self.silenciado() {
            (HUECO + DURACION) * 2.0
        } else {
            0.0
        };
        (
            lista::ANCHO,
            self.y_bloque() + BLOQUE + duraciones + GRUPO + lista::MARGEN,
        )
    }

    /// Lo que ocupa la lista. Vacía sigue midiendo una fila: es donde va el
    /// «No hay notificaciones», y sin ese hueco la tarjeta daría un salto al
    /// llegar la primera.
    // 0.1.0
    fn alto_lista(&self) -> f32 {
        let filas = self.geometria_filas();
        match filas.last() {
            Some((y, alto)) => y + alto,
            None => FILA_NOTIF,
        }
    }

    /// La `y` donde empieza la lista: bajo la cabecera y dentro del grupo.
    // 0.1.0
    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::BAJO_CABECERA + GRUPO
    }

    /// Cambia lo que se enseña. Lo llama el shell cuando la cola cambia.
    // 0.1.0
    pub fn actualizar(&mut self, nueva: Vec<crate::notificaciones::Notificacion>) {
        self.lista = nueva;
        // Lo señalado se apaga: la fila que había bajo el puntero puede haber
        // desaparecido, y dejar el realce puesto marcaría a la de al lado.
        self.fila.señalar(None);
        self.seleccionada = self
            .seleccionada
            .filter(|i| *i < self.lista.len().min(VISIBLES));
        if !self.lista.iter().any(|n| Some(n.id) == self.desplegada) {
            self.desplegada = None;
        }
    }

    /// Qué fila de la lista cae en un punto.
    // 0.1.0
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < lista::MARGEN + GRUPO || x > lista::ANCHO - lista::MARGEN - GRUPO {
            return None;
        }
        let rel = y - self.y_lista();
        if rel < 0.0 {
            return None;
        }
        self.geometria_filas()
            .iter()
            .position(|(arriba, alto)| (*arriba..=arriba + alto).contains(&rel))
    }

    /// El «Borrar todo» de la cabecera. Se mide desde el borde derecho porque
    /// es donde está anclado, igual que se dibuja.
    // 0.1.0
    fn rect_borrar(&self) -> iced_core::Rectangle {
        let ancho = control::ancho_chip(BORRAR);
        iced_core::Rectangle {
            x: lista::ANCHO - lista::MARGEN - 4.0 - ancho,
            y: lista::MARGEN + (lista::CABECERA - control::CHIP) / 2.0,
            width: ancho,
            height: control::CHIP,
        }
    }

    /// El «Config» de la cabecera, a la izquierda del «Borrar todo».
    // 0.1.0
    fn rect_config(&self) -> iced_core::Rectangle {
        let borrar = self.rect_borrar();
        let ancho = control::ancho_chip("Config");
        iced_core::Rectangle {
            x: borrar.x - ENTRE_CHIPS - ancho,
            width: ancho,
            ..borrar
        }
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("notificaciones")
    }

    /// ¿Está el silencio puesto ahora mismo?
    ///
    /// Se calcula en vez de guardarse porque caduca solo: un temporizador que
    /// despierte al shell a la hora en punto para apagar un booleano es
    /// exactamente el despertar de más que este compositor evita.
    // 0.1.0
    pub fn silenciado(&self) -> bool {
        match self.silencio {
            None => false,
            Some((_, None)) => true,
            Some((desde, Some(cuanto))) => desde.elapsed() < cuanto,
        }
    }

    /// El rectángulo del interruptor.
    ///
    /// Va en la fila de «No molestar» y no en la cabecera, que es donde estaba:
    /// allí parecía apagar las notificaciones enteras —al lado de «Borrar
    /// todo»— cuando lo que hace es silenciarlas.
    // 0.1.0
    fn rect_interruptor(&self) -> iced_core::Rectangle {
        iced_core::Rectangle {
            x: lista::ANCHO - lista::MARGEN - GRUPO - 8.0 - lista::INTERRUPTOR_ANCHO,
            y: self.y_bloque() + (BLOQUE - lista::INTERRUPTOR_ALTO) / 2.0,
            width: lista::INTERRUPTOR_ANCHO,
            height: lista::INTERRUPTOR_ALTO,
        }
    }

    /// La `y` donde empieza la fila de «No molestar»: tras la lista entera y
    /// el divisor. Contaba una sola fila, y con dos notificaciones o más el
    /// interruptor se dibujaba más abajo de donde se pulsaba.
    // 0.1.0
    fn y_bloque(&self) -> f32 {
        self.y_lista() + self.alto_lista() + lista::PIE_AIRE
    }

    /// La `y` donde empieza la rejilla de duraciones.
    // 0.1.0
    fn y_duraciones(&self) -> f32 {
        self.y_bloque() + BLOQUE + HUECO
    }

    /// Qué botón de duración cae en un punto.
    // 0.1.0
    fn duracion_en(&self, x: f32, y: f32) -> Option<usize> {
        if !self.silenciado() {
            return None;
        }
        let y0 = self.y_duraciones();
        if x < lista::MARGEN + GRUPO || x > lista::ANCHO - lista::MARGEN - GRUPO || y < y0 {
            return None;
        }
        let rel = y - y0;
        let fila = (rel / (DURACION + HUECO)) as usize;
        if fila > 1 || rel - fila as f32 * (DURACION + HUECO) > DURACION {
            return None;
        }
        let columna = usize::from(x > lista::ANCHO / 2.0);
        Some(fila * 2 + columna)
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| self.duracion_en(x, y));
        let sobre = punto.is_some_and(|(x, y)| {
            self.rect_interruptor()
                .contains(iced_core::Point::new(x, y))
        });
        // Los tres `señalar` **siempre**, sin cortocircuito: pasar de una fila
        // de la lista a un botón de duración tiene que apagar la una y
        // encender el otro.
        let cambio = self.señalada.señalar(señalada);
        let cambio = self
            .fila
            .señalar(punto.and_then(|(x, y)| self.fila_en(x, y)))
            || cambio;
        if sobre == self.sobre_interruptor {
            return cambio;
        }
        self.sobre_interruptor = sobre;
        true
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.fila.animando() || self.interruptor.animando()
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        // La ✕ queda en el extremo derecho. Si la aplicación ofrece acciones,
        // tocar el cuerpo ejecuta la primera (la habitual es «Abrir»), y el
        // teclado puede hacer lo mismo con Intro.
        if let Some(i) = self.fila_en(x, y) {
            if let Some(clave) = self.boton_en(i, x, y) {
                return Some(Accion::NotificacionAccion {
                    id: self.lista[i].id,
                    clave,
                });
            }
            let notif = &self.lista[i];
            // La ✕ solo vale en la línea de arriba de la fila: desplegada, el
            // borde derecho de los renglones de abajo es texto.
            let (arriba, _) = self.geometria_filas()[i];
            if x >= lista::ANCHO - lista::MARGEN - CIERRE
                && y - self.y_lista() - arriba <= FILA_NOTIF
            {
                return Some(Accion::CerrarNotificacion(notif.id));
            }
            if Self::desplegable(notif) {
                self.desplegada = (self.desplegada != Some(notif.id)).then_some(notif.id);
                return None;
            }
            if let Some(accion) = notif.acciones.first() {
                return Some(Accion::NotificacionAccion {
                    id: notif.id,
                    clave: accion.clave.clone(),
                });
            }
            return Some(Accion::CerrarNotificacion(notif.id));
        }
        if self.rect_borrar().contains(punto) {
            return Some(Accion::BorrarNotificaciones);
        }
        if self.rect_config().contains(punto) {
            return Some(Accion::Lanzar(
                "bookos-settings --page notificaciones".into(),
            ));
        }
        if self.rect_interruptor().contains(punto) {
            self.silencio = if self.silenciado() {
                None
            } else {
                Some((Instant::now(), DURACIONES[self.duracion].1))
            };
            self.interruptor.ir_a(self.silenciado() as u8 as f32);
            return None;
        }
        if let Some(i) = self.duracion_en(x, y) {
            self.duracion = i;
            // Elegir una duración enciende el silencio: nadie pulsa «4 horas»
            // para dejarlo apagado.
            self.silencio = Some((Instant::now(), DURACIONES[i].1));
            self.interruptor.ir_a(1.0);
        }
        None
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            crate::TeclaPulsada::Arriba | crate::TeclaPulsada::Abajo => {
                let n = self.lista.len().min(VISIBLES);
                if n == 0 {
                    return Tecla::Ignorada;
                }
                let actual = self.seleccionada.unwrap_or_else(|| {
                    if matches!(tecla, crate::TeclaPulsada::Arriba) {
                        n - 1
                    } else {
                        0
                    }
                });
                let siguiente = if matches!(tecla, crate::TeclaPulsada::Arriba) {
                    actual.saturating_sub(1)
                } else {
                    (actual + 1).min(n - 1)
                };
                self.seleccionada = Some(siguiente);
                self.fila.señalar(Some(siguiente));
                Tecla::Consumida
            }
            crate::TeclaPulsada::Inicio => {
                if self.lista.is_empty() {
                    Tecla::Ignorada
                } else {
                    self.seleccionada = Some(0);
                    self.fila.señalar(Some(0));
                    Tecla::Consumida
                }
            }
            crate::TeclaPulsada::Fin => {
                let n = self.lista.len().min(VISIBLES);
                if n == 0 {
                    Tecla::Ignorada
                } else {
                    self.seleccionada = Some(n - 1);
                    self.fila.señalar(Some(n - 1));
                    Tecla::Consumida
                }
            }
            crate::TeclaPulsada::Intro => {
                let Some(i) = self.seleccionada else {
                    return Tecla::Ignorada;
                };
                let Some(notif) = self.lista.get(i) else {
                    return Tecla::Ignorada;
                };
                // Con el teclado, el primer Intro despliega y el segundo hace
                // lo que haría el botón principal: sin ratón no hay otro modo
                // de llegar a los botones.
                if Self::desplegable(notif) {
                    if self.esta_desplegada(notif)
                        && let Some((clave, _)) = Self::botones(notif).into_iter().next()
                    {
                        return Tecla::Hacer(Accion::NotificacionAccion {
                            id: notif.id,
                            clave,
                        });
                    }
                    self.desplegada = (self.desplegada != Some(notif.id)).then_some(notif.id);
                    return Tecla::Consumida;
                }
                if let Some(accion) = notif.acciones.first() {
                    Tecla::Hacer(Accion::NotificacionAccion {
                        id: notif.id,
                        clave: accion.clave.clone(),
                    })
                } else {
                    Tecla::Hacer(Accion::CerrarNotificacion(notif.id))
                }
            }
            _ => Tecla::Ignorada,
        }
    }

    /// Una notificación de la lista: icono, quién avisa y cuándo, el resumen y
    /// su cuerpo en pequeño. La ✕ solo aparece con el puntero encima, como en
    /// las miniaturas de escritorio: cuatro cruces permanentes compiten con lo
    /// único que hay que leer, que es el texto.
    // 0.1.0
    fn fila_notificacion<'a>(
        &self,
        notif: &'a crate::notificaciones::Notificacion,
        señalada: f32,
    ) -> PanelElement<'a> {
        let ancho = lista::BALDOSA;
        let dibujo: PanelElement<'a> = match &notif.icono {
            Some(ic) => crate::icono::ver(ic, 26.0, 26.0),
            None => Space::new().width(Length::Fixed(26.0)).into(),
        };
        // La crítica lleva el nombre en rojo: es lo que la separa de las demás
        // sin meter un fondo de color que taparía su propio icono.
        let repeticiones = self.lista.iter().filter(|n| n.app == notif.app).count();
        let app = if repeticiones > 1 {
            format!("{} · {}", notif.app, repeticiones)
        } else {
            notif.app.clone()
        };
        let color_app = if notif.critica {
            tema::rojo()
        } else {
            tema::TEXTO2
        };
        let indicador = match (
            notif.acciones.len(),
            notif.progreso,
            notif.progreso_indeterminado,
        ) {
            (acciones, Some(porcentaje), _) if acciones > 0 => {
                format!("{} · {}%", acciones, porcentaje)
            }
            (acciones, _, true) if acciones > 0 => format!("{} · …", acciones),
            (0, Some(porcentaje), _) => format!("{}%", porcentaje),
            (0, _, true) => "…".to_string(),
            (1, _, _) => "1 acción".to_string(),
            (acciones, _, _) if acciones > 0 => format!("{} acciones", acciones),
            _ => String::new(),
        };
        // La hora y la ✕ comparten el mismo sitio, en el borde derecho de la
        // fila: al señalarla la ✕ ocupa el lugar de la hora, como en macOS. Antes
        // la ✕ tenía su propio hueco a la derecha, y la hora quedaba flotando
        // a media fila, lejos del borde.
        let derecha: PanelElement<'a> = if señalada > 0.02 {
            match crate::icono::propio("cerrar") {
                Some(ic) => {
                    crate::icono::ver_teñido_propio(&ic, 12.0, tema::alfa(tema::texto(), señalada))
                }
                None => Space::new().into(),
            }
        } else {
            text(notif.hace()).size(11.0).color(tema::TEXTO2).into()
        };

        // Anchos fijos en toda la cabecera: sin ellos iced reparte lo que sobra
        // entre los tres textos y un nombre de aplicación largo aplastaba la
        // hora hasta partirla en tres líneas («hace / 44 / min»), con lo que la
        // cabecera crecía y empujaba el resumen fuera de la fila. El nombre se
        // recorta a lo que quede libre.
        let ancho_indicador = indicador.chars().count() as f32 * 5.5;
        let ancho_app = (ANCHO_TEXTO - HORA_ANCHO - 6.0 - ancho_indicador - HUECO).max(40.0);
        let cabecera = row![
            container(
                text(lista::recortar(&app, ancho_app, 11.0))
                    .size(11.0)
                    .color(color_app)
            )
            .width(Length::Fixed(ancho_app)),
            Space::new().width(Length::Fill),
            text(indicador).size(10.0).color(tema::TEXTO2),
            Space::new().width(Length::Fixed(6.0)),
            container(derecha)
                .width(Length::Fixed(HORA_ANCHO))
                .align_x(Horizontal::Right),
        ]
        .align_y(Vertical::Center);

        let ancho_texto = ANCHO_TEXTO;
        let resumen = text(lista::recortar(&notif.resumen, ancho_texto, 13.0))
            .size(13.0)
            .font(control::peso(Weight::Medium))
            .color(tema::texto());
        let desplegada = self.esta_desplegada(notif);
        let mut textos = if desplegada {
            // Alturas fijas: el alto de la fila se calcula sin medir texto y
            // tiene que coincidir con lo que se dibuja aquí.
            column![
                container(cabecera).height(Length::Fixed(ALTO_CAB)),
                container(resumen).height(Length::Fixed(ALTO_RES)),
            ]
        } else {
            column![cabecera, resumen]
        };
        if desplegada {
            for linea in Self::detalle(notif) {
                textos = textos.push(
                    container(text(linea).size(11.0).color(tema::TEXTO2))
                        .height(Length::Fixed(LINEA_DET)),
                );
            }
            let botones = Self::botones(notif);
            if !botones.is_empty() {
                let n = botones.len();
                let ancho_boton = (ancho_texto - HUECO * (n as f32 - 1.0)) / n as f32;
                let fila =
                    botones
                        .into_iter()
                        .enumerate()
                        .fold(row![], |fila, (k, (_, etiqueta))| {
                            let primario = k == 0;
                            fila.push(
                                container(
                                    text(lista::recortar(&etiqueta, ancho_boton - 12.0, 12.0))
                                        .size(12.0)
                                        .font(control::peso(Weight::Semibold))
                                        .color(if primario {
                                            tema::sobre_acento()
                                        } else {
                                            tema::texto()
                                        }),
                                )
                                .width(Length::Fixed(ancho_boton))
                                .height(Length::Fixed(ALTO_ACCION))
                                .center_x(Length::Fixed(ancho_boton))
                                .center_y(Length::Fixed(ALTO_ACCION))
                                .style(move |_| container::Style {
                                    background: Some(if primario {
                                        tema::acento().into()
                                    } else {
                                        tema::hover().into()
                                    }),
                                    border: Border {
                                        radius: tema::R_BOTON_PEQUENO.into(),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                            )
                            .push(
                                Space::new().width(Length::Fixed(if k + 1 < n {
                                    HUECO
                                } else {
                                    0.0
                                })),
                            )
                        });
                textos = textos
                    .push(Space::new().height(Length::Fixed(HUECO)))
                    .push(fila);
            }
        } else if !notif.cuerpo.is_empty() {
            textos = textos.push(
                text(lista::recortar(&notif.cuerpo, ancho_texto, 11.0))
                    .size(11.0)
                    .color(tema::TEXTO2),
            );
        }

        let fondo = control::baldosa(señalada);
        // Los textos van en una caja de ancho fijo: sin ella, iced los deja
        // crecer hasta el hueco libre y **parte el resumen en dos líneas**, que
        // desborda una fila de 52 px. Recortar no basta, porque lo recortado se
        // mide a ojo y el envoltorio ocurre después.
        let textos = container(textos).width(Length::Fixed(ancho_texto));
        let alto = self.alto_fila(notif);
        // Desplegada, el icono se queda arriba con el título en vez de flotar a
        // media altura del cuerpo.
        let (alineado, padding) = if desplegada {
            (Vertical::Top, [PAD_V, 12.0])
        } else {
            (Vertical::Center, [0.0, 12.0])
        };
        container(row![dibujo, Space::new().width(Length::Fixed(12.0)), textos,].align_y(alineado))
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .center_y(Length::Fixed(alto))
            .padding(padding)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.0
    fn boton_duracion(&self, i: usize) -> PanelElement<'_> {
        let (etiqueta, _) = DURACIONES[i];
        let elegido = self.duracion == i && self.silenciado();
        let señalado = self.señalada.intensidad(i);
        // La elegida como lo seleccionado de cualquier lista: acento al 10 % y
        // texto en acento. En sólido pesaba más que el interruptor de encima,
        // que es la decisión de verdad.
        let (fondo, color, peso) = if elegido {
            (
                control::seleccionada(señalado),
                tema::acento(),
                Weight::Semibold,
            )
        } else {
            (control::baldosa(señalado), tema::texto(), Weight::Medium)
        };
        let ancho = (lista::BALDOSA - HUECO) / 2.0;
        container(
            text(etiqueta)
                .size(13.0)
                .font(control::peso(peso))
                .color(color),
        )
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(DURACION))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(DURACION))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Una fila de dos botones de duración con su hueco en medio.
    // 0.1.0
    fn fila_duraciones(&self, i: usize) -> PanelElement<'_> {
        row![
            self.boton_duracion(i),
            Space::new().width(Length::Fixed(HUECO)),
            self.boton_duracion(i + 1),
        ]
        .into()
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let cabecera = container(
            row![
                control::titulo("Notificaciones"),
                Space::new().width(Length::Fill),
                control::chip("Config", tema::superficie(), tema::texto()),
                Space::new().width(Length::Fixed(ENTRE_CHIPS)),
                // Con la lista vacía se apaga en vez de desaparecer: «Config»
                // se ancla a él, y el rojo de destruir sin nada que destruir
                // se lee como un aviso.
                if self.lista.is_empty() {
                    control::chip(BORRAR, tema::superficie(), tema::alfa(tema::TEXTO2, 0.6))
                } else {
                    control::chip(BORRAR, tema::alfa(tema::rojo(), 0.14), tema::rojo())
                },
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::ANCHO - lista::MARGEN * 2.0))
        .height(Length::Fixed(lista::CABECERA))
        .padding([0, 4])
        .center_y(Length::Fixed(lista::CABECERA));

        let mut cola = column![].spacing(HUECO);
        if self.lista.is_empty() {
            cola = cola.push(
                container(lista::vacia("No hay notificaciones"))
                    .height(Length::Fixed(FILA_NOTIF))
                    .center_y(Length::Fixed(FILA_NOTIF)),
            );
        }
        for (i, notif) in self.lista.iter().take(VISIBLES).enumerate() {
            let resaltado = self.seleccionada.is_some_and(|seleccion| seleccion == i);
            let intensidad = self
                .fila
                .intensidad(i)
                .max(if resaltado { 0.65 } else { 0.0 });
            cola = cola.push(self.fila_notificacion(notif, intensidad));
        }

        let no_molestar = container(
            row![
                column![
                    text("No molestar")
                        .size(14.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto()),
                    text("Las notificaciones solo se guardan aquí")
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
                Space::new().width(Length::Fill),
                interruptor(self.interruptor.valor()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::BALDOSA))
        .height(Length::Fixed(BLOQUE))
        .center_y(Length::Fixed(BLOQUE))
        .padding([0, 8]);

        let mut grupo = column![
            cola,
            Space::new().height(Length::Fixed(HUECO + 2.0)),
            container(control::divisor(lista::BALDOSA - 12.0)).padding([0, 6]),
            Space::new().height(Length::Fixed(2.0 + HUECO)),
            no_molestar,
        ];
        if self.silenciado() {
            grupo = grupo
                .push(Space::new().height(Length::Fixed(HUECO)))
                .push(self.fila_duraciones(0))
                .push(Space::new().height(Length::Fixed(HUECO)))
                .push(self.fila_duraciones(2));
        }
        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(lista::BAJO_CABECERA)),
            lista::grupo(grupo.into()),
        ];
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El interruptor enciende y apaga el silencio.
    #[test]
    fn el_interruptor_alterna_el_silencio() {
        let mut n = Notificaciones::new();
        assert!(!n.silenciado());
        let r = n.rect_interruptor();
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(n.silenciado());
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(!n.silenciado(), "el segundo toque lo apaga");
    }

    /// Las duraciones solo existen con el silencio puesto —es lo que dice el
    /// diseño—, y elegir una lo mantiene con esa cuenta.
    #[test]
    fn las_duraciones_solo_estan_con_el_silencio_puesto() {
        let mut n = Notificaciones::new();
        let y = n.y_duraciones() + 2.0;
        n.pulsar(30.0, y);
        assert!(!n.silenciado(), "sin silencio ahí no hay ningún botón");

        let r = n.rect_interruptor();
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(n.silenciado());
        n.pulsar(30.0, n.y_duraciones() + 2.0);
        assert_eq!(n.duracion, 0, "el primero es «1 hora»");
        assert!(n.silenciado());
    }

    /// Un silencio caducado no silencia. Se comprueba con una duración de cero,
    /// que es lo único que se puede medir sin esperar una hora.
    #[test]
    fn el_silencio_caduca_solo() {
        let mut n = Notificaciones::new();
        n.silencio = Some((Instant::now(), Some(Duration::ZERO)));
        assert!(!n.silenciado());
    }

    #[test]
    fn las_flechas_y_intro_recuperan_una_accion() {
        let mut n = Notificaciones::new();
        n.actualizar(vec![crate::notificaciones::Notificacion::nueva_con_datos(
            7,
            "Prueba".into(),
            "Aviso".into(),
            String::new(),
            "",
            false,
            vec![crate::notificaciones::Accion {
                clave: "abrir".into(),
                etiqueta: "Abrir".into(),
            }],
            None,
            false,
        )]);
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Abajo),
            Tecla::Consumida
        ));
        // Con acciones, el primer Intro despliega y el segundo la ejecuta.
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Intro),
            Tecla::Consumida
        ));
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::NotificacionAccion { id: 7, .. })
        ));
    }

    fn con_cuerpo(
        id: u32,
        cuerpo: &str,
        acciones: Vec<crate::notificaciones::Accion>,
    ) -> crate::notificaciones::Notificacion {
        crate::notificaciones::Notificacion::nueva_con_datos(
            id,
            "App".into(),
            "Resumen".into(),
            cuerpo.into(),
            "",
            false,
            acciones,
            None,
            false,
        )
    }

    /// Un cuerpo largo se despliega al pulsar y se recoge al volver a pulsar; la
    /// tarjeta crece y encoge con él.
    #[test]
    fn pulsar_una_fila_larga_la_despliega_y_la_recoge() {
        let mut n = Notificaciones::new();
        n.actualizar(vec![con_cuerpo(1, &"palabra ".repeat(40), vec![])]);
        let cerrada = n.size().1;
        let (x, y) = (100.0, n.y_lista() + 10.0);
        assert!(n.pulsar(x, y).is_none(), "desplegar no es una acción");
        assert!(n.size().1 > cerrada);
        assert!(n.pulsar(x, y).is_none());
        assert_eq!(n.size().1, cerrada);
    }

    /// Una fila corta y sin acciones sigue como antes: un toque la cierra.
    #[test]
    fn una_fila_corta_se_sigue_cerrando_con_un_toque() {
        let mut n = Notificaciones::new();
        n.actualizar(vec![con_cuerpo(7, "hola", vec![])]);
        let y = n.y_lista() + 10.0;
        assert!(matches!(
            n.pulsar(100.0, y),
            Some(Accion::CerrarNotificacion(7))
        ));
    }

    /// Desplegada, cada botón devuelve su acción; «default» sale como «Abrir».
    #[test]
    fn los_botones_desplegados_devuelven_su_accion() {
        let accion = |c: &str, e: &str| crate::notificaciones::Accion {
            clave: c.into(),
            etiqueta: e.into(),
        };
        let mut n = Notificaciones::new();
        n.actualizar(vec![con_cuerpo(
            3,
            "hola",
            vec![accion("default", ""), accion("r", "Responder")],
        )]);
        n.pulsar(100.0, n.y_lista() + 10.0);
        let notif = &n.lista[0];
        let y = n.y_lista()
            + PAD_V
            + ALTO_CAB
            + ALTO_RES
            + Notificaciones::detalle(notif).len() as f32 * LINEA_DET
            + HUECO
            + ALTO_ACCION / 2.0;
        let x0 = lista::MARGEN + GRUPO + 50.0;
        let clave = |x: f32| match n.pulsar_sin_efecto(x, y) {
            Some(c) => c,
            None => String::new(),
        };
        assert_eq!(clave(x0 + 5.0), "r");
        assert_eq!(clave(x0 + ANCHO_TEXTO - 5.0), "default");
    }
}
