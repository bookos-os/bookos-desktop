//! El emergente de Sonido, colgado del icono del volumen.
//!
//! Dos secciones, Altavoces y Micrófono, cada una con su etiqueta y su valor
//! encima, y debajo el icono, la píldora gruesa de la referencia de Figma y el
//! botón redondo que silencia. Las medidas son las de las demás tarjetas del
//! panel: 336 de ancho y 16 de margen.
//!
//! **El deslizador es lo primero del shell que se arrastra.** Hasta ahora las
//! emergentes solo respondían a pulsaciones sueltas: el menú y el calendario se
//! recorren y se pulsan, y con eso basta. Una píldora necesita saber que sigue
//! agarrada mientras mueves el ratón, incluso si te sales de la tarjeta — que
//! es lo que hace cualquiera al llevar el volumen al máximo de un tirón.
//!
//! El servicio compartido recibe los cambios de volumen y publica el estado.
//! El hilo de dibujo solo consulta la caché y encola operaciones.

use iced_core::Length;
use iced_core::alignment::Vertical;
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::control::{
    self, BAJO_ETIQUETA, BOTON, ETIQUETA, FILA_PILDORA, HUECO, ICONO, MARGEN_AGARRE, PILDORA,
};
use super::lista::{ANCHO, BAJO_CABECERA, CABECERA, MARGEN};
use super::{Ancla, Tecla};

/// Cuánto más se meten las secciones que la tarjeta: las deja a 20 del borde,
/// alineadas con el título.
const SANGRIA: f32 = 4.0;
/// Ancho del bloque de las secciones.
const BLOQUE: f32 = ANCHO - (MARGEN + SANGRIA) * 2.0;
/// Separación entre las dos secciones.
const ENTRE_SECCIONES: f32 = 14.0;
/// Lo que ocupa una sección: etiqueta, su aire y la fila de la píldora.
const SECCION: f32 = ETIQUETA + BAJO_ETIQUETA + FILA_PILDORA;

/// Cuánto sube o baja el volumen una muesca de rueda.
const PASO_RUEDA: i32 = 5;

/// Qué fila se está tocando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fila {
    Salida,
    Entrada,
}

pub struct Sonido {
    salida: Canal,
    entrada: Canal,
    /// Qué píldora está agarrada ahora mismo, si alguna.
    agarrada: Option<(Fila, u8, bool)>,
    /// El botón de silencio bajo el puntero: 0 la salida, 1 la entrada.
    boton: tema::Realce,
}

/// Un destino de PipeWire tal y como lo enseña el emergente.
struct Canal {
    /// El icono de la izquierda, que dice qué es y no cómo está: el altavoz con
    /// ondas o el micrófono. El estado lo lleva el botón.
    icono_fijo: Option<crate::icono::Icono>,
    disponible: bool,
    nivel: u8,
    silenciado: bool,
    /// El nombre del destino para `wpctl`.
    destino: &'static str,
    /// Si es el micrófono. Decide qué familia de iconos usa.
    micro: bool,
    /// El icono del botón, ya cargado. Vive aquí y no se rasteriza en `view`
    /// porque el elemento que devuelve `ver_teñido` presta del `Icono`: si se
    /// carga dentro de la vista, muere antes de dibujarse.
    icono: Option<crate::icono::Icono>,
}

impl Canal {
    // 0.1.0
    fn nuevo(destino: &'static str, micro: bool) -> Self {
        let (nivel, silenciado) = crate::widgets::volumen::consultar(destino).unwrap_or((0, false));
        let mut canal = Self {
            disponible: crate::widgets::volumen::consultar(destino).is_some(),
            nivel,
            silenciado,
            destino,
            micro,
            icono_fijo: crate::icono::propio(if micro { "micro" } else { "volumen-alto" }),
            icono: None,
        };
        canal.actualizar_icono();
        canal
    }

    /// El nombre del icono que le toca al estado actual.
    // 0.1.0
    fn nombre_icono(&self) -> &'static str {
        match (self.micro, self.silenciado, self.nivel) {
            (true, true, _) => "micro-silencio",
            (true, false, _) => "micro",
            (_, true, _) | (_, _, 0) => "volumen-silencio",
            (_, _, n) if n < 40 => "volumen-bajo",
            (_, _, n) if n < 75 => "volumen-medio",
            _ => "volumen-alto",
        }
    }

    // 0.1.0
    fn actualizar_icono(&mut self) {
        self.icono = crate::icono::propio(self.nombre_icono());
    }

    /// Manda el nivel a PipeWire. No espera: ver la cabecera del módulo.
    // 0.1.0
    fn poner(&mut self, nivel: u8, notificar: bool) {
        if !self.disponible {
            return;
        }
        self.nivel = nivel.min(100);
        // Mover el deslizador de un canal silenciado lo devuelve a la vida: es
        // lo que hace el plasmoide, y lo que espera cualquiera que empuje la
        // barra para volver a oír algo.
        if self.silenciado && self.nivel > 0 {
            self.silenciado = false;
            // El volumen que va justo detrás confirma las dos cosas; si esta
            // orden avisara también, una sola muesca produciría dos OSD y dos
            // sonidos.
            let _ = lanzar(&["set-mute", self.destino, "0"], false);
        }
        self.enviar_volumen(notificar);
        self.actualizar_icono();
    }

    // 0.1.0
    fn enviar_volumen(&self, notificar: bool) {
        let _ = lanzar(
            &[
                "set-volume",
                self.destino,
                &format!("{:.2}", self.nivel as f32 / 100.0),
            ],
            notificar,
        );
    }

    // 0.1.0
    fn alternar_silencio(&mut self) {
        if !self.disponible {
            return;
        }
        self.silenciado = !self.silenciado;
        let _ = lanzar(&["set-mute", self.destino, "toggle"], true);
        self.actualizar_icono();
    }

    // 0.1.0
    fn etiqueta(&self) -> String {
        if !self.disponible {
            "No disponible".into()
        } else if self.silenciado {
            "Silenciado".into()
        } else {
            format!("{} %", self.nivel)
        }
    }
}

impl Sonido {
    // 0.1.0
    pub fn refrescar(&mut self) -> bool {
        if self.agarrada.is_some() {
            return false;
        }
        let mut changed = false;
        for canal in [&mut self.salida, &mut self.entrada] {
            let value = bookos_system::volume(canal.micro);
            if canal.disponible != value.is_some() {
                canal.disponible = value.is_some();
                changed = true;
            }
            if let Some((nivel, silenciado)) = value
                && (canal.nivel != nivel || canal.silenciado != silenciado)
            {
                canal.nivel = nivel;
                canal.silenciado = silenciado;
                canal.actualizar_icono();
                changed = true;
            }
        }
        changed
    }

    // 0.1.0
    pub fn new() -> Self {
        Self {
            salida: Canal::nuevo("@DEFAULT_AUDIO_SINK@", false),
            entrada: Canal::nuevo("@DEFAULT_AUDIO_SOURCE@", true),
            agarrada: None,
            boton: tema::Realce::nuevo(),
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (
            ANCHO,
            MARGEN * 2.0 + CABECERA + BAJO_CABECERA + SECCION * 2.0 + ENTRE_SECCIONES,
        )
    }

    /// Cuelga del icono del volumen, como el calendario cuelga del reloj.
    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("volumen")
    }

    /// El rectángulo de la píldora de una fila, relativo a la emergente.
    // 0.1.0
    fn rect_pildora(&self, fila: Fila) -> iced_core::Rectangle {
        let y0 = MARGEN + CABECERA + BAJO_CABECERA;
        let y = match fila {
            Fila::Salida => y0,
            Fila::Entrada => y0 + SECCION + ENTRE_SECCIONES,
        } + ETIQUETA
            + BAJO_ETIQUETA;
        debug_assert!(
            y + FILA_PILDORA <= self.size().1,
            "la fila se sale de la tarjeta"
        );
        iced_core::Rectangle {
            x: MARGEN + SANGRIA + ICONO + HUECO,
            y: y + (FILA_PILDORA - PILDORA) / 2.0,
            width: control::ancho_pildora(BLOQUE, true),
            height: PILDORA,
        }
    }

    /// El rectángulo del botón de silencio de una fila.
    // 0.1.0
    fn rect_boton(&self, fila: Fila) -> iced_core::Rectangle {
        let p = self.rect_pildora(fila);
        iced_core::Rectangle {
            x: p.x + p.width + HUECO,
            y: p.y - (FILA_PILDORA - PILDORA) / 2.0,
            width: BOTON,
            height: BOTON,
        }
    }

    /// El chip «Config» de la cabecera, pegado a la derecha.
    // 0.1.0
    fn rect_config(&self) -> iced_core::Rectangle {
        let ancho = control::ancho_chip("Config");
        iced_core::Rectangle {
            x: ANCHO - MARGEN - SANGRIA - ancho,
            y: MARGEN + (CABECERA - control::CHIP) / 2.0,
            width: ancho,
            height: control::CHIP,
        }
    }

    // 0.1.0
    fn canal(&mut self, fila: Fila) -> &mut Canal {
        match fila {
            Fila::Salida => &mut self.salida,
            Fila::Entrada => &mut self.entrada,
        }
    }

    /// De una `x` dentro de la píldora al nivel que le toca.
    // 0.1.0
    fn nivel_en(&self, fila: Fila, x: f32) -> u8 {
        let r = self.rect_pildora(fila);
        control::nivel_en(x, r.x, r.width)
    }

    /// ¿Hay una píldora agarrada? Mientras la haya, el puntero le llega aunque
    /// se salga de la tarjeta.
    // 0.1.0
    pub fn agarrado(&self) -> bool {
        self.agarrada.is_some()
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        // Con una píldora agarrada, el puntero es el deslizador y nada más: el
        // ratón puede estar sobre un botón mientras se arrastra, y encenderlo
        // ahí sería prometer una pulsación que no va a ocurrir.
        if let (Some((fila, _, _)), Some((x, _))) = (self.agarrada, punto) {
            let nivel = self.nivel_en(fila, x);
            if self.canal(fila).nivel == nivel {
                return false;
            }
            self.canal(fila).poner(nivel, false);
            return true;
        }
        let sobre = punto.and_then(|(x, y)| {
            let p = iced_core::Point::new(x, y);
            [Fila::Salida, Fila::Entrada]
                .into_iter()
                .position(|f| self.rect_boton(f).contains(p))
        });
        self.boton.señalar(sobre)
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.boton.animando()
    }

    // añadido en la 0.2.0 -- distinguir el deslizador evita confirmar un botón al convertir el toque en arrastre.
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        [Fila::Salida, Fila::Entrada].into_iter().any(|fila| {
            let mut zona = self.rect_pildora(fila);
            zona.y -= MARGEN_AGARRE;
            zona.height += MARGEN_AGARRE * 2.0;
            zona.contains(iced_core::Point::new(x, y))
        })
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        if self.rect_config().contains(punto) {
            return Some(Accion::Lanzar("bookos-settings --page sonido".into()));
        }
        for fila in [Fila::Salida, Fila::Entrada] {
            if self.rect_boton(fila).contains(punto) {
                self.canal(fila).alternar_silencio();
                return None;
            }
            // La zona agarrable es más alta que la píldora: apuntar a 24 px de
            // alto con el ratón en movimiento falla más de lo que parece.
            let mut zona = self.rect_pildora(fila);
            zona.y -= MARGEN_AGARRE;
            zona.height += MARGEN_AGARRE * 2.0;
            if zona.contains(punto) {
                let canal = self.canal(fila);
                self.agarrada = Some((fila, canal.nivel, canal.silenciado));
                let nivel = self.nivel_en(fila, x);
                self.canal(fila).poner(nivel, false);
                return None;
            }
        }
        None
    }

    /// Soltar el botón deja de arrastrar. Devuelve si hay que repintar.
    // 0.1.0
    pub fn soltar(&mut self) -> bool {
        let Some((fila, nivel_inicial, silencio_inicial)) = self.agarrada.take() else {
            return false;
        };
        let canal = self.canal(fila);
        if canal.nivel != nivel_inicial || canal.silenciado != silencio_inicial {
            canal.enviar_volumen(true);
        }
        true
    }

    /// La rueda sobre una fila sube y baja su nivel, como en el plasmoide.
    // 0.1.0
    pub fn desplazar(&mut self, _dx: f32, dy: f32) -> bool {
        if dy == 0.0 {
            return false;
        }
        let paso = if dy > 0.0 { PASO_RUEDA } else { -PASO_RUEDA };
        let nivel = (self.salida.nivel as i32 + paso).clamp(0, 100) as u8;
        if nivel == self.salida.nivel {
            return false;
        }
        self.salida.poner(nivel, true);
        true
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda | T::Abajo => {
                let n = (self.salida.nivel as i32 - PASO_RUEDA).clamp(0, 100) as u8;
                self.salida.poner(n, true);
                Tecla::Consumida
            }
            T::Derecha | T::Arriba => {
                let n = (self.salida.nivel as i32 + PASO_RUEDA).clamp(0, 100) as u8;
                self.salida.poner(n, true);
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let cabecera = container(
            row![
                control::titulo("Sonido"),
                Space::new().width(Length::Fill),
                control::chip("Config", tema::superficie(), tema::texto()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(CABECERA))
        .padding([0, SANGRIA as u16])
        .center_y(Length::Fixed(CABECERA));
        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(BAJO_CABECERA)),
            container(column![
                self.seccion("Altavoces", &self.salida, self.boton.intensidad(0)),
                Space::new().height(Length::Fixed(ENTRE_SECCIONES)),
                self.seccion("Micrófono", &self.entrada, self.boton.intensidad(1)),
            ])
            .padding([0, SANGRIA as u16]),
        ];
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }

    /// Una sección: etiqueta y valor arriba; icono, píldora y botón abajo.
    // 0.1.0
    fn seccion<'a>(
        &'a self, titulo: &'a str, canal: &'a Canal, señalado: f32
    ) -> PanelElement<'a> {
        let cabecera = control::etiqueta(titulo, canal.etiqueta(), BLOQUE);
        if !canal.disponible {
            let mensaje = bookos_system::unavailable("audio")
                .map(|_| "Servicio de audio no disponible")
                .unwrap_or("Esperando dispositivo de audio…");
            return column![
                cabecera,
                Space::new().height(Length::Fixed(BAJO_ETIQUETA)),
                container(text(mensaje).size(12.0).color(tema::TEXTO2))
                    .height(Length::Fixed(FILA_PILDORA))
                    .center_y(Length::Fixed(FILA_PILDORA)),
            ]
            .into();
        }
        // El botón va en acento mientras el canal suena y en el color del surco
        // al silenciarlo: su icono es el del estado, así que dice las dos cosas.
        let controles = control::fila_pildora(
            canal.icono_fijo.as_ref(),
            control::pildora(
                control::ancho_pildora(BLOQUE, true),
                PILDORA,
                canal.nivel,
                canal.silenciado,
                tema::superficie(),
            ),
            Some(control::boton(
                canal.icono.as_ref(),
                !canal.silenciado,
                tema::superficie(),
                señalado,
            )),
        );
        column![
            cabecera,
            Space::new().height(Length::Fixed(BAJO_ETIQUETA)),
            controles,
        ]
        .into()
    }
}

// 0.1.0
fn lanzar(args: &[&str], notificar: bool) -> bool {
    bookos_system::audio_request(args, notificar)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El nivel sale de dónde se pulsa dentro de la píldora, y los extremos
    /// tienen que dar 0 y 100 exactos: si el máximo se queda en 98, no hay
    /// forma de poner el volumen al tope arrastrando.
    #[test]
    fn la_pildora_reparte_el_nivel_de_extremo_a_extremo() {
        let s = Sonido::new();
        let r = s.rect_pildora(Fila::Salida);
        assert_eq!(s.nivel_en(Fila::Salida, r.x), 0);
        assert_eq!(s.nivel_en(Fila::Salida, r.x + r.width), 100);
        assert_eq!(s.nivel_en(Fila::Salida, r.x + r.width / 2.0), 50);
        // Y fuera de la barra no se desborda: arrastrar más allá del borde deja
        // el nivel en el tope, no en un número imposible.
        assert_eq!(s.nivel_en(Fila::Salida, r.x - 500.0), 0);
        assert_eq!(s.nivel_en(Fila::Salida, r.x + r.width + 500.0), 100);
    }

    /// Las dos filas no se solapan y caben en la tarjeta.
    #[test]
    fn las_dos_filas_caben_y_no_se_pisan() {
        let s = Sonido::new();
        let (ancho, alto) = s.size();
        let a = s.rect_boton(Fila::Salida);
        let b = s.rect_boton(Fila::Entrada);
        assert!(a.y + a.height <= b.y, "las filas se solapan");
        assert!(b.y + b.height <= alto, "la segunda fila se sale por abajo");
        assert!(a.x + a.width <= ancho, "el botón se sale por la derecha");
    }

    /// Soltar sin haber agarrado nada no obliga a repintar.
    #[test]
    fn soltar_sin_agarrar_no_hace_nada() {
        let mut s = Sonido::new();
        assert!(!s.soltar());
        assert!(!s.agarrado());
    }
}
