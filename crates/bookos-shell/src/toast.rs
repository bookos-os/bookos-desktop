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

use crate::notificaciones::Notificacion;
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
const ALTO_TITULOS: f32 = 34.0;
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
const HUECO_BOTONES: f32 = 6.0;
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
}

/// Lo que se ha pulsado dentro de un aviso.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulsado {
    /// El botón de una acción, con su clave.
    Boton(String),
    /// La ✕: quitarlo también de la lista.
    Cerrar,
}

impl Toast {
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
            return tema::C_ENTRADA.eval(tema::fraccion(t, ENTRADA));
        }
        self.partida.map_or(1.0, |p| p.0) * (1.0 - self.avance_salida())
    }

    // 0.1.1
    fn avance_salida(&self) -> f32 {
        let t = self.desde.elapsed().saturating_sub(self.quieto);
        1.0 - tema::C_ENTRADA.eval(1.0 - tema::fraccion(t, SALIDA))
    }

    // 0.1.1
    pub fn desplazamiento_horizontal(&self) -> f32 {
        if tema::efectos_reducidos() {
            return 0.0;
        }
        if self.saliendo() {
            let origen = self.partida.map_or(0.0, |p| p.2);
            return origen + (RECORRIDO - origen) * self.avance_salida();
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
        let mut alto = MARGEN * 2.0 + ICONO.max(ALTO_TITULOS + LINEA_CUERPO * lineas);
        if n.progreso.is_some() || n.progreso_indeterminado {
            alto += ALTO_PROGRESO;
        }
        if !self.botones().is_empty() {
            alto += RESPIRO + ALTO_BOTON;
        }
        alto
    }

    /// Cuánto más abajo asoma que el de delante, en lógicos. El zoom del
    /// compositor crece desde el centro, así que además de bajar `ASOMO` por
    /// nivel hay que compensar lo que la tarjeta encogida sube su base.
    // 0.1.0
    pub fn desplazamiento(&self) -> f32 {
        let nivel = self.profundidad as f32;
        nivel * ASOMO + self.alto_tarjeta() * (1.0 - self.encogido()) / 2.0
    }

    // 0.1.0
    fn encogido(&self) -> f32 {
        1.0 - ENCOGE_POR_NIVEL * self.profundidad as f32
    }

    /// Alto de la tarjeta, sin el margen de la sombra.
    // 0.1.0
    pub fn alto_tarjeta(&self) -> f32 {
        self.size().1 - MARGEN_SOMBRA * 2.0
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
            0.94 + 0.06 * tema::C_MUELLE_POPOVER.eval(tema::fraccion(t, ENTRADA))
        };
        entrada * self.encogido()
    }

    /// El temporizador despierta al empezar la salida; desde ahí hacen falta
    /// fotogramas hasta acabarla aunque no haya entrada del usuario.
    // 0.1.0
    // modified 0.1.1 -- cuenta también la salida
    pub fn animando(&self) -> bool {
        let t = self.desde.elapsed();
        t < ENTRADA || (t >= self.quieto && t < self.quieto + SALIDA)
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
        self.alto() - MARGEN - ALTO_BOTON
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (
            ANCHO + MARGEN_SOMBRA * 2.0,
            self.alto() + MARGEN_SOMBRA * 2.0,
        )
    }

    /// Lo pulsado en un punto, en coordenadas relativas a la tarjeta (sin el
    /// margen reservado para la sombra). Solo el de delante tiene algo que
    /// pulsar: los de atrás no enseñan contenido.
    // 0.1.0
    pub fn accion_en(&self, x: f32, y: f32) -> Option<Pulsado> {
        if self.profundidad > 0 {
            return None;
        }
        let x_cierre = ANCHO - CIERRE_BORDE - CIERRE;
        if (x_cierre..=x_cierre + CIERRE).contains(&x)
            && (CIERRE_BORDE..=CIERRE_BORDE + CIERRE).contains(&y)
        {
            return Some(Pulsado::Cerrar);
        }
        let botones = self.botones();
        let y_botones = self.y_botones();
        if botones.is_empty() || !(y_botones..=y_botones + ALTO_BOTON).contains(&y) {
            return None;
        }
        Self::huecos_botones(botones.len())
            .zip(botones)
            .find(|((x0, ancho), _)| (*x0..=x0 + ancho).contains(&x))
            .map(|(_, a)| Pulsado::Boton(a.clave.clone()))
    }

    // modified 0.1.1 -- tarjeta compacta: app en gris arriba y resumen en negrita
    pub fn view(&self) -> PanelElement<'_> {
        if self.profundidad > 0 {
            return self.vista_de_atras();
        }
        let n = &self.notificacion;
        let dibujo: PanelElement<'_> = match &n.icono {
            Some(ic) => crate::icono::ver(ic, ICONO, ICONO),
            None => Space::new().width(Length::Fixed(ICONO)).into(),
        };
        let ancho_texto = ANCHO - MARGEN * 2.0;
        // Sin pisar la ✕, que cuelga de la esquina de la cabecera.
        let ancho_titulos = ancho_texto - ICONO - 12.0 - CIERRE - 8.0;
        // Quién avisa, pequeño y en gris, y debajo el resumen en negrita: lo
        // que se lee de un vistazo es qué pasa, no qué app lo dice. En rojo si
        // es crítica: es lo único que la distingue de las demás sin meterle un
        // fondo de color que taparía su icono.
        let mut titulos = column![
            text(crate::emergente::recortar_texto(
                &n.app,
                ancho_titulos,
                12.0
            ))
            .size(12.0)
            .color(tema::TEXTO2),
            text(crate::emergente::recortar_texto(
                &n.resumen,
                ancho_titulos,
                14.0
            ))
            .size(14.0)
            .font(Font {
                weight: Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(if n.critica {
                tema::rojo()
            } else {
                tema::texto()
            }),
        ];
        // modificado en la 0.2.0 -- el cuerpo va bajo el resumen y no bajo el icono
        // Debajo del resumen y no de toda la cabecera: colgado del margen
        // empezaba bajo el icono, desalineado del título, y además sumaba un
        // respiro y un renglón de alto que el aviso no necesita.
        if !self.cuerpo.is_empty() {
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
        .align_y(Vertical::Center);
        let mut contenido = column![cabecera];
        if let Some(valor) = n.progreso {
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
                        background: Some(tema::surco().into()),
                        border: Border {
                            radius: 2.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
        } else if n.progreso_indeterminado {
            // Ocupa lo mismo que la barra: `alto()` reserva ese hueco.
            contenido = contenido
                .push(Space::new().height(Length::Fixed(RESPIRO - 4.0)))
                .push(text("En curso…").size(10.0).color(tema::TEXTO2));
        }
        let acciones = self.botones();
        if !acciones.is_empty() {
            let huecos: Vec<_> = Self::huecos_botones(acciones.len()).collect();
            // Solo la primera acción es primaria; las demás van en el fondo de
            // hover, como el botón secundario del sistema (AI-DESIGN-SYSTEM §4).
            // Con todos en acento no había un botón principal que mirar.
            let botones = acciones.iter().zip(huecos).enumerate().fold(
                row![],
                |fila, (i, (accion, (_, ancho)))| {
                    let primario = i == 0;
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
        let cuerpo = container(contenido)
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(alto))
            .padding(MARGEN as u16);
        let tarjeta = container(stack![cuerpo, Self::cierre()])
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(alto))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    ..Default::default()
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

    /// La ✕ de la esquina: gris y discreta, porque la roja es la de cerrar
    /// ventanas y esta solo quita un aviso.
    // 0.1.0
    fn cierre() -> PanelElement<'static> {
        let aspa: PanelElement<'static> = match crate::icono::propio("cerrar") {
            Some(ic) => crate::icono::ver_teñido_propio(&ic, CIERRE * 0.55, tema::TEXTO2),
            None => Space::new().into(),
        };
        container(
            container(aspa)
                .center_x(Length::Fixed(CIERRE))
                .center_y(Length::Fixed(CIERRE))
                .style(|_| container::Style {
                    background: Some(tema::hover().into()),
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
        let nivel = self.profundidad as f32;
        let tarjeta = container(Space::new())
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(self.alto()))
            .style(move |_| container::Style {
                background: Some(tema::mezclar(tema::card(), Color::BLACK, 0.15 * nivel).into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(tarjeta).padding(MARGEN_SOMBRA).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boton(clave: &str) -> Pulsado {
        Pulsado::Boton(clave.into())
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
        let t = Toast::new(
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
        let t = Toast::new(
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
        assert_eq!(t.accion_en(x, y), Some(Pulsado::Cerrar));
        t.colocar(1, Some(t.alto_propio()));
        assert_eq!(t.accion_en(x, y), None);
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
}
