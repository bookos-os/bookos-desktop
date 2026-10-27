//! El teclado en pantalla.
//!
//! Lo dibuja el shell, dentro del compositor, como el panel y el dock: así
//! comparte tokens, acento y animaciones sin un proceso más ni un cliente
//! input-method aparte. El shell no sabe de xkb ni de Wayland: entrega
//! [`Accion`]es con el carácter que se quiere escribir y el compositor busca
//! la tecla física que lo produce en la distribución puesta.
//!
//! La geometría de las teclas sale de una sola función, [`Teclado::teclas`],
//! que usan tanto el dibujo como el hit-test. Si cada uno la calculara por su
//! lado, a la primera corrección de un margen los toques caerían en la tecla de
//! al lado.
//!
//! ## Colores
//!
//! Como Gboard con Material You: el teclado se tiñe del acento, que con el
//! acento automático es el color dominante del fondo (`tema::paleta_automatica`).
//! Todo se mezcla **opaco**: un relleno translúcido en tiny-skia cuesta del
//! orden de 17 veces uno opaco, y el teclado es una superficie grande que se
//! repinta en cada pulsación.

use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Column, Row, Space, container, stack, text};

use crate::icono;
use crate::tema;
use crate::view::PanelElement;

/// Alto de una tecla. 48 lógicos es el objetivo táctil mínimo de Material
/// (48 dp); por debajo, en la fila de letras se tocaba la de al lado.
const ALTO_TECLA: f32 = 48.0;
/// Hueco entre teclas, en las dos direcciones.
const HUECO: f32 = 8.0;
/// Margen alrededor del bloque de teclas.
const MARGEN: f32 = 8.0;
/// La franja de arriba, con el botón de ocultar.
const FRANJA: f32 = 36.0;
/// El cuerpo de las letras. Mayor que `T_TITULO`: se lee a un brazo de
/// distancia y con el dedo encima tapando parte de la tecla.
const T_TECLA: f32 = 22.0;
/// Lo que hay que mantener una tecla para que escriba su variante (á por a).
/// Es el `ViewConfiguration.getLongPressTimeout` de Android.
const PULSACION_LARGA: Duration = Duration::from_millis(400);
/// Dos toques en mayúsculas dentro de este tiempo la dejan fija.
const DOBLE_TOQUE: Duration = Duration::from_millis(350);

/// Alto total del teclado, en lógicos.
pub const ALTO: f32 = MARGEN + FRANJA + HUECO + 4.0 * ALTO_TECLA + 3.0 * HUECO + MARGEN;

/// Lo que el compositor tiene que escribir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Salida {
    Caracter(char),
    Intro,
    Retroceso,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accion {
    /// Pulsar y soltar.
    Escribe(Salida),
    /// Pulsar sin soltar: el retroceso se mantiene mientras el dedo está
    /// encima y el propio cliente lo repite, con el ritmo de repetición del
    /// teclado físico. Así no hace falta un temporizador aquí.
    Pulsa(Salida),
    Suelta(Salida),
    Ocultar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capa {
    Letras,
    Simbolos,
    MasSimbolos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mayus {
    No,
    /// Solo la siguiente letra.
    Una,
    Fija,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tecla {
    Car(char),
    Mayus,
    Borrar,
    Intro,
    Espacio,
    Capa(Capa),
    Ocultar,
}

/// Una tecla colocada, en lógicos dentro del teclado.
#[derive(Debug, Clone, Copy)]
struct Colocada {
    tecla: Tecla,
    x: f32,
    y: f32,
    ancho: f32,
    alto: f32,
}

impl Colocada {
    /// El hit-test incluye la mitad del hueco de cada lado: un toque que cae
    /// entre dos teclas es de la más cercana, no de nadie.
    fn contiene(&self, x: f32, y: f32) -> bool {
        let m = HUECO / 2.0;
        x >= self.x - m
            && x < self.x + self.ancho + m
            && y >= self.y - m
            && y < self.y + self.alto + m
    }
}

/// Cada fila con el ancho de cada tecla en unidades de tecla de letra.
fn filas(capa: Capa) -> [Vec<(Tecla, f32)>; 4] {
    let cars = |s: &str| s.chars().map(|c| (Tecla::Car(c), 1.0)).collect::<Vec<_>>();
    let con = |ini: (Tecla, f32), medio: Vec<(Tecla, f32)>, fin: (Tecla, f32)| {
        std::iter::once(ini)
            .chain(medio)
            .chain(std::iter::once(fin))
            .collect()
    };
    let abajo = |cambio: Capa| {
        vec![
            (Tecla::Capa(cambio), 1.5),
            (Tecla::Car(','), 1.0),
            (Tecla::Espacio, 5.0),
            (Tecla::Car('.'), 1.0),
            (Tecla::Intro, 1.5),
        ]
    };
    match capa {
        Capa::Letras => [
            cars("qwertyuiop"),
            cars("asdfghjklñ"),
            con((Tecla::Mayus, 1.5), cars("zxcvbnm"), (Tecla::Borrar, 1.5)),
            abajo(Capa::Simbolos),
        ],
        Capa::Simbolos => [
            cars("1234567890"),
            cars("@#€_&-+()/"),
            con(
                (Tecla::Capa(Capa::MasSimbolos), 1.5),
                cars("*\"':;!?"),
                (Tecla::Borrar, 1.5),
            ),
            abajo(Capa::Letras),
        ],
        Capa::MasSimbolos => [
            cars("[]{}<>^%=\\"),
            cars("~`|$£¥°¿¡·"),
            con(
                (Tecla::Capa(Capa::Simbolos), 1.5),
                cars("ºª¬«»§¨"),
                (Tecla::Borrar, 1.5),
            ),
            abajo(Capa::Letras),
        ],
    }
}

/// La variante que escribe una pulsación larga, la que en un teclado español
/// se busca más. Una sola y no un menú: elegir entre cinco con el dedo encima
/// es otra interacción entera, y en castellano casi siempre se quiere la aguda.
fn variante(c: char) -> Option<char> {
    Some(match c {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ó',
        'u' => 'ú',
        'n' => 'ñ',
        'c' => 'ç',
        '?' => '¿',
        '!' => '¡',
        '-' => '—',
        _ => return None,
    })
}

/// Los colores del teclado, del acento y el tema de ahora.
///
/// Las proporciones son a ojo contra capturas de Gboard con Material You: el
/// fondo apenas teñido, las teclas de letra casi neutras y las de acción con
/// el tono claramente a la vista, para que se distingan sin leer el icono.
struct Colores {
    fondo: Color,
    tecla: Color,
    accion: Color,
    pulsada: Color,
    intro: Color,
    sobre_intro: Color,
    tinta: Color,
}

impl Colores {
    fn ahora() -> Self {
        let acento = tema::acento();
        let fondo = tema::mezclar(tema::bg(), acento, 0.10);
        let tecla = tema::mezclar(tema::card(), acento, 0.06);
        let accion = tema::mezclar(tema::card(), acento, 0.28);
        Self {
            fondo,
            tecla,
            accion,
            pulsada: tema::mezclar(tecla, tema::texto(), 0.14),
            intro: acento,
            sobre_intro: tema::sobre_acento(),
            tinta: tema::texto(),
        }
    }
}

pub struct Teclado {
    /// Ancho lógico de la pantalla: el teclado va de lado a lado.
    ancho: f32,
    capa: Capa,
    mayus: Mayus,
    /// Cuándo se tocó mayúsculas por última vez, para el doble toque.
    mayus_en: Option<Instant>,
    /// La tecla bajo el dedo y desde cuándo.
    pulsada: Option<(Tecla, Instant)>,
    /// 0 escondido, 1 a la vista. El compositor lo usa para deslizarlo.
    visible: tema::Transicion,
}

impl Teclado {
    pub fn new(ancho: f32, capa: Capa) -> Self {
        let mut visible = tema::Transicion::nueva(0.0, tema::D_TARJETA, tema::C_ENTRADA);
        visible.ir_a(1.0);
        Self {
            ancho,
            capa,
            mayus: Mayus::No,
            mayus_en: None,
            pulsada: None,
            visible,
        }
    }

    pub fn size(&self) -> (f32, f32) {
        (self.ancho, ALTO)
    }

    /// Vuelve a entrar si se estaba yendo, con la capa que pida el campo.
    pub fn mostrar(&mut self, capa: Capa) {
        if self.visible.destino() < 1.0 {
            self.capa = capa;
            self.visible.ir_a(1.0);
        }
    }

    pub fn ocultar(&mut self) {
        self.pulsada = None;
        self.visible.ir_a(0.0);
    }

    /// Se está yendo o ya se fue.
    pub fn saliendo(&self) -> bool {
        self.visible.destino() < 1.0
    }

    /// Ya no se ve y se puede tirar.
    pub fn terminado(&self) -> bool {
        self.saliendo() && !self.visible.animando()
    }

    pub fn animando(&self) -> bool {
        self.visible.animando()
    }

    /// Cuánto asoma, en lógicos: lo que tapa de la pantalla ahora mismo.
    pub fn asoma(&self) -> f32 {
        ALTO * self.visible.valor()
    }

    fn teclas(&self) -> Vec<Colocada> {
        let bloque = self.ancho - 2.0 * MARGEN;
        let x0 = (self.ancho - bloque) / 2.0;
        // La unidad sale de la fila de diez letras, que es la que llena el
        // bloque; las demás se centran con lo que les sobre.
        let unidad = (bloque - 9.0 * HUECO) / 10.0;
        let mut colocadas = vec![Colocada {
            tecla: Tecla::Ocultar,
            x: x0 + bloque - FRANJA * 1.5,
            y: MARGEN,
            ancho: FRANJA * 1.5,
            alto: FRANJA,
        }];
        for (i, fila) in filas(self.capa).iter().enumerate() {
            let anchos: Vec<f32> = fila
                .iter()
                .map(|(_, u)| u * unidad + (u - 1.0).max(0.0) * HUECO)
                .collect();
            let total: f32 = anchos.iter().sum::<f32>() + (fila.len() - 1) as f32 * HUECO;
            let y = MARGEN + FRANJA + HUECO + i as f32 * (ALTO_TECLA + HUECO);
            let mut x = x0 + (bloque - total) / 2.0;
            for ((tecla, _), ancho) in fila.iter().zip(anchos) {
                colocadas.push(Colocada {
                    tecla: *tecla,
                    x,
                    y,
                    ancho,
                    alto: ALTO_TECLA,
                });
                x += ancho + HUECO;
            }
        }
        colocadas
    }

    fn tecla_en(&self, x: f32, y: f32) -> Option<Tecla> {
        self.teclas()
            .into_iter()
            .find(|c| c.contiene(x, y))
            .map(|c| c.tecla)
    }

    /// Un dedo baja en `(x, y)`, en lógicos relativos al teclado.
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let tecla = self.tecla_en(x, y)?;
        self.pulsada = Some((tecla, Instant::now()));
        (tecla == Tecla::Borrar).then_some(Accion::Pulsa(Salida::Retroceso))
    }

    /// El dedo se levanta. La tecla que cuenta es la que se pulsó, no la que
    /// haya debajo ahora: un dedo que resbala medio milímetro sobre el borde
    /// escribiría la letra de al lado.
    pub fn soltar(&mut self) -> Option<Accion> {
        let (tecla, desde) = self.pulsada.take()?;
        let larga = desde.elapsed() >= PULSACION_LARGA;
        match tecla {
            Tecla::Borrar => Some(Accion::Suelta(Salida::Retroceso)),
            Tecla::Intro => Some(Accion::Escribe(Salida::Intro)),
            Tecla::Espacio => Some(Accion::Escribe(Salida::Caracter(' '))),
            Tecla::Ocultar => Some(Accion::Ocultar),
            Tecla::Capa(capa) => {
                self.capa = capa;
                None
            }
            Tecla::Mayus => {
                let doble = self.mayus_en.is_some_and(|t| t.elapsed() < DOBLE_TOQUE);
                self.mayus = match (self.mayus, doble) {
                    (Mayus::Una, true) => Mayus::Fija,
                    (Mayus::No, _) => Mayus::Una,
                    (Mayus::Una | Mayus::Fija, _) => Mayus::No,
                };
                self.mayus_en = Some(Instant::now());
                None
            }
            Tecla::Car(c) => {
                let c = if larga { variante(c).unwrap_or(c) } else { c };
                let c = match self.mayus {
                    Mayus::No => c,
                    Mayus::Una | Mayus::Fija => c.to_uppercase().next().unwrap_or(c),
                };
                if self.mayus == Mayus::Una {
                    self.mayus = Mayus::No;
                }
                Some(Accion::Escribe(Salida::Caracter(c)))
            }
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let colores = Colores::ahora();
        let pulsada = self.pulsada.map(|(t, _)| t);
        let teclas = self.teclas();

        // Las filas se montan con las posiciones de `teclas()`: cada una es
        // una fila de iced con el hueco delante de la primera tecla, así que
        // lo dibujado cae exactamente donde mira el hit-test.
        let mut columna = Column::new();
        let mut y_previo = 0.0;
        for fila in teclas.chunk_by(|a, b| a.y == b.y) {
            let primera = fila[0];
            columna = columna.push(Space::new().height(Length::Fixed(primera.y - y_previo)));
            let mut r = Row::new();
            let mut x_previo = 0.0;
            for c in fila {
                r = r
                    .push(Space::new().width(Length::Fixed(c.x - x_previo)))
                    .push(self.tecla(c, &colores, pulsada == Some(c.tecla)));
                x_previo = c.x + c.ancho;
            }
            columna = columna.push(r);
            y_previo = primera.y + primera.alto;
        }

        container(columna)
            .width(Length::Fixed(self.ancho))
            .height(Length::Fixed(ALTO))
            .into()
    }

    fn tecla<'a>(&self, c: &Colocada, colores: &Colores, pulsada: bool) -> PanelElement<'a> {
        let (fondo, tinta) = match c.tecla {
            Tecla::Intro => (colores.intro, colores.sobre_intro),
            Tecla::Ocultar => (colores.fondo, colores.tinta),
            Tecla::Car(_) | Tecla::Espacio => (colores.tecla, colores.tinta),
            Tecla::Mayus | Tecla::Borrar | Tecla::Capa(_) => (colores.accion, colores.tinta),
        };
        let fondo = if pulsada {
            tema::mezclar(fondo, colores.pulsada, 0.6)
        } else {
            fondo
        };
        let lado_icono = 22.0;
        let contenido: PanelElement<'a> = match c.tecla {
            Tecla::Car(ch) => {
                let ch = match self.mayus {
                    Mayus::No => ch,
                    _ => ch.to_uppercase().next().unwrap_or(ch),
                };
                let letra: PanelElement<'a> =
                    text(ch.to_string()).size(T_TECLA).color(tinta).into();
                // La variante, pequeña arriba a la derecha: sin la pista nadie
                // sabe que manteniendo la «a» sale la «á».
                match variante(ch.to_lowercase().next().unwrap_or(ch)) {
                    Some(v) => stack![
                        container(letra).center(Length::Fill),
                        container(
                            text(v.to_string())
                                .size(tema::T_PEQUENO)
                                .color(tema::TEXTO2)
                        )
                        .width(Length::Fill)
                        .align_x(Horizontal::Right)
                        .padding([2.0, 6.0]),
                    ]
                    .into(),
                    None => letra,
                }
            }
            Tecla::Espacio => Space::new().into(),
            Tecla::Capa(capa) => text(match capa {
                Capa::Letras => "ABC",
                Capa::Simbolos => "?123",
                Capa::MasSimbolos => "=\\<",
            })
            .size(tema::T_CUERPO)
            .font(Font {
                weight: Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(tinta)
            .into(),
            Tecla::Mayus => {
                let nombre = if self.mayus == Mayus::Fija {
                    "mayus-fija"
                } else {
                    "mayus"
                };
                // Encendida, la flecha va en acento: es como se ve de un
                // vistazo que la siguiente letra saldrá en mayúscula.
                let color = if self.mayus == Mayus::No {
                    tinta
                } else {
                    tema::enlace()
                };
                self.icono(nombre, lado_icono, color)
            }
            Tecla::Borrar => self.icono("borrar", lado_icono, tinta),
            Tecla::Intro => self.icono("intro", lado_icono, tinta),
            Tecla::Ocultar => self.icono("chevron-abajo", 20.0, tema::TEXTO2),
        };
        container(contenido)
            .width(Length::Fixed(c.ancho))
            .height(Length::Fixed(c.alto))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn icono<'a>(&self, nombre: &str, lado: f32, color: Color) -> PanelElement<'a> {
        match icono::propio(nombre) {
            Some(ic) => icono::ver_teñido_propio(&ic, lado, color),
            None => Space::new().into(),
        }
    }

    /// El color con el que se limpia el buffer: el del fondo del teclado.
    pub fn fondo(&self) -> Color {
        Colores::ahora().fondo
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn centro(t: &Teclado, buscada: Tecla) -> (f32, f32) {
        let c = t
            .teclas()
            .into_iter()
            .find(|c| c.tecla == buscada)
            .expect("la tecla está en la capa");
        (c.x + c.ancho / 2.0, c.y + c.alto / 2.0)
    }

    #[test]
    fn cada_tecla_se_encuentra_en_su_centro() {
        let t = Teclado::new(1440.0, Capa::Letras);
        for c in t.teclas() {
            let (x, y) = (c.x + c.ancho / 2.0, c.y + c.alto / 2.0);
            assert_eq!(t.tecla_en(x, y), Some(c.tecla));
        }
    }

    #[test]
    fn las_filas_de_letras_aprovechan_todo_el_ancho_horizontal() {
        for ancho in [1280.0, 1440.0, 1920.0] {
            let t = Teclado::new(ancho, Capa::Letras);
            let teclas = t.teclas();
            for fila in teclas[1..].chunk_by(|a, b| a.y == b.y) {
                let primera = fila.first().expect("fila con teclas");
                let ultima = fila.last().expect("fila con teclas");
                assert!((primera.x - MARGEN).abs() < 0.01);
                assert!((ultima.x + ultima.ancho - (ancho - MARGEN)).abs() < 0.01);
                for tecla in fila {
                    assert_eq!(
                        t.tecla_en(tecla.x + tecla.ancho / 2.0, tecla.y + tecla.alto / 2.0),
                        Some(tecla.tecla)
                    );
                }
            }
        }
    }

    #[test]
    fn las_teclas_caben_en_el_teclado() {
        for ancho in [360.0, 1280.0, 1440.0, 2560.0] {
            for capa in [Capa::Letras, Capa::Simbolos, Capa::MasSimbolos] {
                let t = Teclado::new(ancho, capa);
                for c in t.teclas() {
                    assert!(
                        c.x >= 0.0 && c.x + c.ancho <= ancho + 0.01,
                        "{c:?} a {ancho}"
                    );
                    assert!(c.y + c.alto <= ALTO + 0.01, "{c:?}");
                }
            }
        }
    }

    #[test]
    fn mayuscula_una_vez_y_fija_con_doble_toque() {
        let mut t = Teclado::new(1280.0, Capa::Letras);
        let (mx, my) = centro(&t, Tecla::Mayus);
        let (ax, ay) = centro(&t, Tecla::Car('a'));
        let mut toca = |t: &mut Teclado, x, y| {
            t.pulsar(x, y);
            t.soltar()
        };
        toca(&mut t, mx, my);
        assert_eq!(
            toca(&mut t, ax, ay),
            Some(Accion::Escribe(Salida::Caracter('A')))
        );
        assert_eq!(
            toca(&mut t, ax, ay),
            Some(Accion::Escribe(Salida::Caracter('a')))
        );
        toca(&mut t, mx, my);
        toca(&mut t, mx, my);
        assert_eq!(
            toca(&mut t, ax, ay),
            Some(Accion::Escribe(Salida::Caracter('A')))
        );
        assert_eq!(
            toca(&mut t, ax, ay),
            Some(Accion::Escribe(Salida::Caracter('A')))
        );
    }

    #[test]
    fn borrar_se_mantiene_hasta_soltar() {
        let mut t = Teclado::new(1280.0, Capa::Letras);
        let (x, y) = centro(&t, Tecla::Borrar);
        assert_eq!(t.pulsar(x, y), Some(Accion::Pulsa(Salida::Retroceso)));
        assert_eq!(t.soltar(), Some(Accion::Suelta(Salida::Retroceso)));
    }
}
