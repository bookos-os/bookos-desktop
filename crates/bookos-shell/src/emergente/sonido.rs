//! El emergente de Sonido, colgado del icono del volumen.
//!
//! Dos tarjetas, Altavoces y Micrófono, con la forma de las de Brillo: título y
//! valor arriba; debajo, el icono que silencia, la barra y el icono de lo que
//! es. Comparten medidas con Brillo porque cuelgan del panel una al lado de la
//! otra.
//!
//! **El deslizador es lo primero del shell que se arrastra.** Hasta ahora las
//! emergentes solo respondían a pulsaciones sueltas: el menú y el calendario se
//! recorren y se pulsan, y con eso basta. Una píldora necesita saber que sigue
//! agarrada mientras mueves el ratón, incluso si te sales de la tarjeta — que
//! es lo que hace cualquiera al llevar el volumen al máximo de un tirón.
//!
//! El servicio compartido recibe los cambios de volumen y publica el estado.
//! El hilo de dibujo solo consulta la caché y encola operaciones.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Length, Padding, Point, Rectangle};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

// Las medidas de la tarjeta de Brillo: las dos cuelgan del panel una al lado
// de la otra y tienen que leerse como la misma familia.
use super::brillo::{
    AJUSTES, ANCHO_BARRA, ANCHO_TARJETA, ENTRE, ENTRE_TARJETAS, PAD_ABAJO, PAD_ARRIBA, PAD_H,
    SANGRIA, TITULO, VALOR,
};
use super::control::{self, ENTRE_BARRA, FILA_BARRA, ICONO_BARRA, PISTA};
use super::lista::{ANCHO, BAJO_CABECERA, CABECERA, MARGEN};
use super::{Ancla, Tecla, t};

/// Alto de la tarjeta de un canal: título y valor, y la barra.
const ALTO_CANAL: f32 = PAD_ARRIBA + TITULO + ENTRE + FILA_BARRA + PAD_ABAJO;

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
    /// Lo pulsable bajo el puntero: 0 y 1 los silencios de salida y
    /// entrada, 2 los ajustes.
    boton: tema::Realce,
    /// Cuánto engorda la barra agarrada, con muelle, como en Brillo.
    grosor: tema::Transicion,
    icono_ajustes: Option<crate::icono::Icono>,
}

/// Un destino de PipeWire tal y como lo enseña el emergente.
struct Canal {
    /// El icono grande de la derecha de la barra, que dice qué es y no cómo
    /// está: el altavoz con ondas o el micrófono. El estado lo lleva el de la
    /// izquierda, que es el botón de silencio.
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
        let estado = crate::widgets::volumen::consultar(destino);
        let (nivel, silenciado) = estado.unwrap_or((0, false));
        let mut canal = Self {
            disponible: estado.is_some(),
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
            t("No disponible", "Unavailable").into()
        } else if self.silenciado {
            t("Silenciado", "Muted").into()
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
            grosor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            icono_ajustes: crate::icono::propio("preferencias"),
        }
    }

    // 0.1.0
    // modified 0.2.2 -- tarjetas por canal, como Brillo
    pub fn size(&self) -> (f32, f32) {
        (
            ANCHO,
            MARGEN * 2.0 + CABECERA + BAJO_CABECERA + ALTO_CANAL * 2.0 + ENTRE_TARJETAS,
        )
    }

    /// Cuelga del icono del volumen, como el calendario cuelga del reloj.
    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("volumen")
    }

    /// Dónde empieza la tarjeta de una fila, relativo a la emergente.
    // añadido en la 0.2.2
    fn y_tarjeta(fila: Fila) -> f32 {
        let y = MARGEN + CABECERA + BAJO_CABECERA;
        match fila {
            Fila::Salida => y,
            Fila::Entrada => y + ALTO_CANAL + ENTRE_TARJETAS,
        }
    }

    /// La fila de la barra con sus iconos: lo que se agarra.
    // añadido en la 0.2.2
    fn y_fila(fila: Fila) -> f32 {
        Self::y_tarjeta(fila) + PAD_ARRIBA + TITULO + ENTRE
    }

    /// La barra de verdad, de [`PISTA`] de alto: de ahí sale el nivel.
    // 0.1.0
    // modified 0.2.2 -- la barra de Brillo
    fn rect_pildora(&self, fila: Fila) -> Rectangle {
        Rectangle {
            x: MARGEN + PAD_H + ICONO_BARRA + ENTRE_BARRA,
            y: Self::y_fila(fila) + (FILA_BARRA - PISTA) / 2.0,
            width: ANCHO_BARRA,
            height: PISTA,
        }
    }

    /// Lo que se puede agarrar: la fila entera de la barra, no solo el surco.
    // añadido en la 0.2.2
    fn zona_barra(&self, fila: Fila) -> Rectangle {
        let mut r = self.rect_pildora(fila);
        r.y = Self::y_fila(fila);
        r.height = FILA_BARRA;
        r
    }

    /// El botón de silencio: el icono pequeño de la izquierda de la barra.
    // 0.1.0
    // modified 0.2.2 -- del botón redondo de la derecha al icono de la izquierda
    fn rect_boton(&self, fila: Fila) -> Rectangle {
        Rectangle {
            x: MARGEN + PAD_H,
            y: Self::y_fila(fila),
            width: ICONO_BARRA,
            height: FILA_BARRA,
        }
    }

    /// El botón de ajustes de la cabecera, el mismo que el de Brillo.
    // 0.1.0
    // modified 0.2.2 -- de un chip «Config» a un botón redondo con su icono
    fn rect_config(&self) -> Rectangle {
        Rectangle {
            x: ANCHO - MARGEN - SANGRIA - AJUSTES,
            y: MARGEN + (CABECERA - AJUSTES) / 2.0,
            width: AJUSTES,
            height: AJUSTES,
        }
    }

    // 0.1.0
    fn canal(&mut self, fila: Fila) -> &mut Canal {
        match fila {
            Fila::Salida => &mut self.salida,
            Fila::Entrada => &mut self.entrada,
        }
    }

    fn disponible(&self, fila: Fila) -> bool {
        match fila {
            Fila::Salida => self.salida.disponible,
            Fila::Entrada => self.entrada.disponible,
        }
    }

    /// De una `x` dentro de la píldora al nivel que le toca.
    // 0.1.0
    fn nivel_en(&self, fila: Fila, x: f32) -> u8 {
        let r = self.rect_pildora(fila);
        control::nivel_barra(x, r.x, r.width)
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
            let p = Point::new(x, y);
            [
                self.rect_boton(Fila::Salida),
                self.rect_boton(Fila::Entrada),
                self.rect_config(),
            ]
            .into_iter()
            .position(|r| r.contains(p))
        });
        self.boton.señalar(sobre)
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.boton.animando() || self.grosor.animando()
    }

    // añadido en la 0.2.0 -- distinguir el deslizador evita confirmar un botón al convertir el toque en arrastre.
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        [Fila::Salida, Fila::Entrada]
            .into_iter()
            .filter(|fila| self.disponible(*fila))
            .any(|fila| self.zona_barra(fila).contains(Point::new(x, y)))
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = Point::new(x, y);
        if self.rect_config().contains(punto) {
            return Some(Accion::Lanzar("bookos-settings --page sonido".into()));
        }
        for fila in [Fila::Salida, Fila::Entrada] {
            if self.rect_boton(fila).contains(punto) {
                self.canal(fila).alternar_silencio();
                return None;
            }
            // La zona agarrable es la fila entera: apuntar a 10 px de alto con
            // el ratón en movimiento falla más de lo que parece.
            if self.disponible(fila) && self.zona_barra(fila).contains(punto) {
                let canal = self.canal(fila);
                self.agarrada = Some((fila, canal.nivel, canal.silenciado));
                self.grosor.ir_a(1.0);
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
        self.grosor.ir_a(0.0);
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
        if dy == 0.0 || !self.salida.disponible {
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
    // modified 0.2.2 -- cabecera y tarjetas de Brillo
    pub fn view(&self) -> PanelElement<'_> {
        let ajustes = {
            let fondo = tema::mezclar(
                tema::superficie_en_tarjeta(),
                tema::elevado(),
                self.boton.intensidad(2),
            );
            let dentro: PanelElement<'_> = match &self.icono_ajustes {
                Some(ic) => crate::icono::ver_teñido(ic, 17.0, Some(tema::texto())),
                None => Space::new().into(),
            };
            container(dentro)
                .width(Length::Fixed(AJUSTES))
                .height(Length::Fixed(AJUSTES))
                .center_x(Length::Fixed(AJUSTES))
                .center_y(Length::Fixed(AJUSTES))
                .style(move |_: &iced_widget::Theme| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: (AJUSTES / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        let cabecera = container(
            row![
                control::titulo(t("Sonido", "Sound")),
                Space::new().width(Length::Fill),
                ajustes
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
            self.tarjeta(t("Altavoces", "Speakers"), Fila::Salida, &self.salida, 0),
            Space::new().height(Length::Fixed(ENTRE_TARJETAS)),
            self.tarjeta(
                t("Micrófono", "Microphone"),
                Fila::Entrada,
                &self.entrada,
                1
            ),
        ];
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }

    /// La tarjeta de un canal: título y valor; debajo, el silencio, la barra y
    /// el icono de lo que es.
    // añadido en la 0.2.2
    fn tarjeta<'a>(
        &'a self,
        titulo: &'a str,
        fila: Fila,
        canal: &'a Canal,
        realce: usize,
    ) -> PanelElement<'a> {
        let cabecera = row![
            text(titulo)
                .size(14.0)
                .font(control::peso(Weight::Medium))
                .color(tema::texto()),
            Space::new().width(Length::Fill),
            container(text(canal.etiqueta()).size(12.0).color(tema::texto2()))
                .width(Length::Fixed(VALOR + 40.0))
                .align_right(Length::Fixed(VALOR + 40.0)),
        ]
        .align_y(Vertical::Center);
        let fila_barra: PanelElement<'a> = if canal.disponible {
            // El silencio se pulsa en su icono, que enseña el estado (tachado
            // al silenciar), con placa al pasar por encima o mientras calla.
            let placa = tema::mezclar(
                tema::superficie_en_tarjeta(),
                tema::elevado(),
                if canal.silenciado {
                    1.0
                } else {
                    self.boton.intensidad(realce)
                },
            );
            let silencio = container(match &canal.icono {
                Some(ic) => crate::icono::ver_teñido(ic, 18.0, Some(tema::texto())),
                None => Space::new().into(),
            })
            .width(Length::Fixed(ICONO_BARRA))
            .height(Length::Fixed(FILA_BARRA))
            .center_x(Length::Fixed(ICONO_BARRA))
            .center_y(Length::Fixed(FILA_BARRA))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(placa.into()),
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            let grueso = if self.agarrada.is_some_and(|(f, _, _)| f == fila) {
                control::grueso_barra(self.grosor.valor())
            } else {
                PISTA
            };
            // Silenciado, el nivel se conserva pero en gris: es lo que volverá
            // a sonar al quitar el silencio.
            let barra = control::barra(
                ANCHO_BARRA,
                grueso,
                f32::from(canal.nivel) / 100.0,
                if canal.silenciado {
                    tema::texto2()
                } else {
                    tema::acento()
                },
                tema::elevado(),
                &[],
                None,
            );
            let fijo = container(match &canal.icono_fijo {
                Some(ic) => crate::icono::ver_teñido(ic, 22.0, Some(tema::texto())),
                None => Space::new().into(),
            })
            .width(Length::Fixed(ICONO_BARRA))
            .height(Length::Fixed(FILA_BARRA))
            .center_x(Length::Fixed(ICONO_BARRA))
            .center_y(Length::Fixed(FILA_BARRA));
            row![
                silencio,
                Space::new().width(Length::Fixed(ENTRE_BARRA)),
                barra,
                Space::new().width(Length::Fixed(ENTRE_BARRA)),
                fijo,
            ]
            .align_y(Vertical::Center)
            .height(Length::Fixed(FILA_BARRA))
            .into()
        } else {
            let mensaje = bookos_system::unavailable("audio")
                .map(|_| {
                    t(
                        "Servicio de audio no disponible",
                        "Audio service unavailable",
                    )
                })
                .unwrap_or(t(
                    "Esperando dispositivo de audio…",
                    "Waiting for an audio device…",
                ));
            container(text(mensaje).size(12.0).color(tema::texto2()))
                .height(Length::Fixed(FILA_BARRA))
                .center_y(Length::Fixed(FILA_BARRA))
                .into()
        };
        container(column![
            container(cabecera)
                .height(Length::Fixed(TITULO))
                .center_y(Length::Fixed(TITULO)),
            Space::new().height(Length::Fixed(ENTRE)),
            fila_barra,
        ])
        .width(Length::Fixed(ANCHO_TARJETA))
        .height(Length::Fixed(ALTO_CANAL))
        .padding(Padding {
            top: PAD_ARRIBA,
            bottom: PAD_ABAJO,
            left: PAD_H,
            right: PAD_H,
        })
        .style(|_: &iced_widget::Theme| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                ..Default::default()
            },
            ..Default::default()
        })
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

    #[test]
    fn canal_no_disponible_no_agarra_ni_desplaza() {
        let mut s = Sonido::new();
        s.salida.disponible = false;
        s.salida.nivel = 40;
        let r = s.zona_barra(Fila::Salida);
        let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(!s.arrastre_en(x, y));
        s.pulsar(x, y);
        assert!(!s.agarrado());
        assert!(!s.grosor.animando());
        assert!(!s.desplazar(0.0, 5.0));
        assert_eq!(s.salida.nivel, 40);
    }
}
