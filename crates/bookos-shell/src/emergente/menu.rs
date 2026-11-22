//! El menú de BookOS: el que cuelga del logo, arriba a la izquierda.
//!
//! Nació como la versión propia del plasmoide `bookos-menu` y conserva sus
//! acciones y su hover en acento sólido con el texto en blanco; las medidas ya
//! no son las suyas sino las del menú de macOS, que es como se quiere ver. Lo que allí ejecuta un
//! `Plasma5Support.DataSource` de tipo "executable", aquí sale como una
//! [`Accion`] que ejecuta el compositor.
//!
//! Las acciones de energía pasan por logind y respetan sus inhibidores: una
//! aplicación que está guardando datos o presentando contenido puede retrasar
//! la transición de forma segura.

use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, FilaAccesible, Tecla, t};

/// Lo que el menú puede crecer para que quepa un nombre largo en «Cerrar
/// sesión de …». Más allá se recorta: un menú de medio panel tapa el escritorio
/// que se quería mirar.
const ANCHO_MIN: f32 = 200.0;
const ANCHO_MAX: f32 = 280.0;
/// 30 y no los 24 que tuvo: a 24 con esquinas de 10 el menú era la única
/// superficie del escritorio con otra densidad y otro radio —dock, tarjetas y
/// bloqueo van a 18–22—, y al lado de ellos se leía como un menú de otro
/// sistema. 30 con letra de 13 deja el mismo aire que las filas del centro de
/// control y cabe la esquina de popover sin comerse la primera fila.
// modified 0.2.0 -- 24 → 30, a juego con el resto de emergentes
pub(super) const FILA: f32 = 30.0;
/// Alto del bloque separador, con su línea de 1 px en medio.
pub(super) const DIVISOR: f32 = 11.0;
/// Margen lateral de las filas dentro del menú.
pub(super) const MARGEN: f32 = 5.0;
/// Márgenes de arriba y de abajo del menú entero: iguales al lateral, para que
/// el realce de la primera y la última fila quede concéntrico con la esquina.
const BORDE_SUP: f32 = MARGEN;
const BORDE_INF: f32 = MARGEN;
/// Relleno del texto dentro de la fila, a cada lado.
pub(super) const RELLENO: f32 = 10.0;
/// Aire mínimo entre la etiqueta y su atajo.
pub(super) const HUECO_ATAJO: f32 = 20.0;
/// Letra de los atajos y relleno lateral de la cápsula de cada tecla.
const T_ATAJO: f32 = tema::T_PEQUENO - 1.0;
const RELLENO_TECLA: f32 = 5.0;
/// Entre una tecla y la siguiente del mismo atajo.
const HUECO_TECLA: f32 = 4.0;

/// Alto del icono de una fila, y hueco hasta el texto.
///
/// 16 y no los 18 del panel: la fila mide 24 px y un icono de 18 la llena de
/// borde a borde, que es lo que hacía que el menú se leyese como una lista de
/// botones en vez de como un menú.
pub(super) const ICONO: f32 = 16.0;
pub(super) const HUECO_ICONO: f32 = 10.0;

/// Una entrada del menú, o un separador.
///
/// Sin icono, a diferencia del menú del dock: en el del logo cada entrada es
/// una frase que se lee de un vistazo, y una columna de pictogramas delante la
/// convertía en un panel de botones. Es lo primero que se nota al ponerlo al
/// lado del de macOS.
enum Entrada {
    Item {
        etiqueta: String,
        atajo: Option<&'static str>,
        accion: fn() -> Accion,
    },
    Divisor,
}

// 0.1.0
// modificado en la 0.2.0 -- sin iconos y «Cerrar sesión de <nombre>» (sin puntos suspensivos desde la 0.2.0)
fn entradas() -> Vec<Entrada> {
    use Entrada::{Divisor, Item};
    let item = |etiqueta: &str, atajo, accion| Item {
        etiqueta: etiqueta.to_string(),
        atajo,
        accion,
    };
    let mut entradas = vec![
        // La ventana es del propio shell, no un programa que lanzar: no hay
        // `bookos-about` en el sistema, y `kinfocenter` —lo que se abría
        // antes— es el panel de otro escritorio y enseña las versiones de
        // Plasma y de Qt en vez de las de BookOS.
        item(t("Acerca de este PC", "About This PC"), None, || Accion::Acerca),
        Divisor,
        // Antes de «Preferencias del sistema», que abre otro programa: esta
        // es del propio escritorio y se aplica al pulsarla.
        item(t("Apariencia…", "Appearance…"), None, || Accion::Emergente("apariencia")),
        item(t("Preferencias del sistema…", "System Preferences…"), None, || {
            Accion::Lanzar("bookos-settings".into())
        }),
        // `bookos-store` todavía no existe —no hay ni un crate con ese nombre
        // en el repositorio—, así que pulsar aquí no hacía nada: `sh -c
        // bookos-store` fallaba con "orden no encontrada" y el menú se cerraba
        // como si hubiera funcionado. Mientras no haya tienda propia, cae a
        // Discover, que es la que ya trae esta base y hace lo mismo de verdad:
        // instalar software.
        item("BookOS Store…", None, || {
            Accion::Lanzar("bookos-store || plasma-discover".into())
        }),
        Divisor,
        item(t("Dormir", "Sleep"), None, || {
            Accion::Lanzar("systemctl suspend".into())
        }),
    ];
    // Debajo de Dormir, y solo si logind dice que se puede: sin swap en disco
    // el botón fallaría sin más.
    if crate::confirmacion::hibernar_disponible() {
        entradas.push(item(t("Hibernar", "Hibernate"), None, || {
            Accion::Lanzar("systemctl hibernate".into())
        }));
    }
    let login = std::env::var("USER").unwrap_or_default();
    let nombre = crate::bloqueo::nombre_completo(&login).unwrap_or(login);
    entradas.extend([
        item(t("Reiniciar…", "Restart…"), None, || {
            Accion::Lanzar("systemctl reboot".into())
        }),
        item(t("Apagar…", "Shut down…"), None, || {
            Accion::Lanzar("systemctl poweroff".into())
        }),
        Divisor,
        item(t("Pantalla de bloqueo", "Lock Screen"), Some("Meta+L"), || Accion::Bloquear),
        Item {
            // Con el nombre, como en macOS: en un equipo con varias cuentas
            // dice cuál se va a cerrar. Sin `USER` queda la frase sola.
            etiqueta: if nombre.is_empty() {
                t("Cerrar sesión…", "Log out…").into()
            } else {
                format!("{} {nombre}…", t("Cerrar sesión de", "Log out"))
            },
            atajo: Some("Ctrl+Alt+Supr"),
            accion: || Accion::CerrarSesion,
        },
    ]);
    entradas
}

/// Lo que ocupa una fila sin el texto: margen y relleno a cada lado.
const RODEO: f32 = (MARGEN + RELLENO) * 2.0;

/// El ancho que pide la entrada más larga, entero para que el buffer caiga en
/// píxel exacto a cualquier escala.
// añadido en la 0.2.0
fn ancho_para(entradas: &[Entrada]) -> f32 {
    let texto = entradas
        .iter()
        .filter_map(|e| match e {
            Entrada::Item {
                etiqueta, atajo, ..
            } => Some(
                crate::widget::ancho_de(etiqueta, tema::T_CUERPO)
                    + atajo.map_or(0.0, |a| HUECO_ATAJO + ancho_atajo(a)),
            ),
            Entrada::Divisor => None,
        })
        .fold(0.0, f32::max);
    (texto + RODEO).clamp(ANCHO_MIN, ANCHO_MAX).ceil()
}

pub struct Menu {
    entradas: Vec<Entrada>,
    ancho: f32,
    /// La entrada bajo el puntero —o la elegida con el teclado— y cuánto le
    /// queda a la anterior para apagarse.
    señalada: tema::Realce,
}

impl Menu {
    // 0.1.0
    // modificado en la 0.2.0 -- ancho según el contenido
    pub fn new() -> Self {
        let mut entradas = entradas();
        let ancho = ancho_para(&entradas);
        // Solo el nombre de la cuenta puede pasarse del tope; se recorta él y
        // no el atajo, que es corto y fijo.
        for entrada in &mut entradas {
            if let Entrada::Item {
                etiqueta,
                atajo: Some(atajo),
                ..
            } = entrada
            {
                let sitio = ancho - RODEO - HUECO_ATAJO - ancho_atajo(atajo);
                // `recortar_texto` mide con un ancho medio por letra y se queda
                // corto: «Cerrar sesión de eve…» cabía de sobra y salía como
                // «Cerrar sesión de …». Solo se recorta lo que de verdad no
                // cabe según la medida real.
                if crate::widget::ancho_de(etiqueta, tema::T_CUERPO) > sitio {
                    *etiqueta = super::recortar_texto(etiqueta, sitio, tema::T_CUERPO);
                }
            }
        }
        Self {
            entradas,
            ancho,
            señalada: tema::Realce::nuevo(),
        }
    }

    /// El realce entrando o saliendo. Mientras dure, el compositor repinta.
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        let alto: f32 = self
            .entradas
            .iter()
            .map(|e| match e {
                Entrada::Item { .. } => FILA,
                Entrada::Divisor => DIVISOR,
            })
            .sum();
        (self.ancho, alto + BORDE_SUP + BORDE_INF)
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        // Pegado al borde izquierdo, bajo el logo del panel.
        Ancla::BajoElPanel { x: 8.0 }
    }

    /// Qué entrada cae en una `y` lógica. Las filas no son todas del mismo alto
    /// —los divisores miden menos— así que se recorre en vez de dividir.
    // 0.1.0
    fn entrada_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho {
            return None;
        }
        let mut cursor = BORDE_SUP;
        for (i, entrada) in self.entradas.iter().enumerate() {
            let alto = match entrada {
                Entrada::Item { .. } => FILA,
                Entrada::Divisor => DIVISOR,
            };
            if y >= cursor && y < cursor + alto {
                // Sobre un divisor no hay nada que señalar.
                return matches!(entrada, Entrada::Item { .. }).then_some(i);
            }
            cursor += alto;
        }
        None
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.señalada
            .señalar(punto.and_then(|(x, y)| self.entrada_en(x, y)))
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.entrada_en(x, y)?;
        match &self.entradas[i] {
            Entrada::Item { accion, .. } => Some(accion()),
            Entrada::Divisor => None,
        }
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Abajo | T::Tabulador => self.mover(1),
            T::Arriba => self.mover(-1),
            T::Inicio => {
                if let Some(i) = self
                    .entradas
                    .iter()
                    .position(|e| matches!(e, Entrada::Item { .. }))
                {
                    self.señalada.señalar(Some(i));
                }
                Tecla::Consumida
            }
            T::Fin => {
                if let Some(i) = self
                    .entradas
                    .iter()
                    .rposition(|e| matches!(e, Entrada::Item { .. }))
                {
                    self.señalada.señalar(Some(i));
                }
                Tecla::Consumida
            }
            // Sin selección, Intro no hace nada: mejor eso que activar una
            // entrada accidentalmente. Con una entrada señalada, la ejecuta.
            T::Intro | T::Caracter(' ') => match self.señalada.actual() {
                Some(i) => match &self.entradas[i] {
                    Entrada::Item { accion, .. } => Tecla::Hacer(accion()),
                    Entrada::Divisor => Tecla::Ignorada,
                },
                None => Tecla::Ignorada,
            },
            _ => Tecla::Ignorada,
        }
    }

    /// Entradas expuestas a AT-SPI2 / Orca.
    // añadido en la 0.2.3
    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let mut cursor = BORDE_SUP;
        let mut items = Vec::new();
        for (i, entrada) in self.entradas.iter().enumerate() {
            match entrada {
                Entrada::Item {
                    etiqueta, atajo, ..
                } => {
                    items.push(FilaAccesible {
                        id: format!("menu:{i}"),
                        rol: accesskit::Role::MenuItem,
                        nombre: etiqueta.clone(),
                        valor: atajo.unwrap_or("").to_string(),
                        rect: (MARGEN, cursor, self.ancho - MARGEN * 2.0, FILA),
                        marcada: None,
                        seleccionada: self.señalada.actual() == Some(i),
                    });
                    cursor += FILA;
                }
                Entrada::Divisor => cursor += DIVISOR,
            }
        }
        items
    }

    /// Mueve la selección saltándose los divisores.
    // 0.1.0
    fn mover(&mut self, paso: isize) -> Tecla {
        let n = self.entradas.len();
        let mut i = match self.señalada.actual() {
            Some(i) => i as isize,
            // Al entrar por teclado se empieza por el extremo que toque.
            None if paso > 0 => -1,
            None => n as isize,
        };
        for _ in 0..n {
            i += paso;
            if i < 0 || i >= n as isize {
                return Tecla::Consumida;
            }
            if matches!(self.entradas[i as usize], Entrada::Item { .. }) {
                self.señalada.señalar(Some(i as usize));
                return Tecla::Consumida;
            }
        }
        Tecla::Consumida
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(Length::Fixed(BORDE_SUP))];
        for (i, entrada) in self.entradas.iter().enumerate() {
            col = col.push(match entrada {
                Entrada::Item {
                    etiqueta, atajo, ..
                } => fila(etiqueta, *atajo, None, self.señalada.intensidad(i)),
                Entrada::Divisor => divisor(),
            });
        }
        col = col.push(Space::new().height(Length::Fixed(BORDE_INF)));

        caja(col.into(), self.ancho)
    }
}

/// La caja translúcida de un menú: fondo de tarjeta y borde.
/// La comparten el menú del logo, el del dock y el del escritorio con su
/// submenú, que antes la escribían cada uno por su lado.
///
/// El radio es el de popover (18), el de todas las emergentes. Tuvo el de
/// botón pequeño (10) mientras las filas medían 24, porque con 18 las esquinas
/// se comían la primera y la última; a 30 de fila ya caben, y con el margen de
/// 5 el realce de radio 13 (`R_ITEM_POPOVER`) queda concéntrico.
// 0.2.0
// modified 0.2.0 -- radio de popover, como el resto de emergentes
pub(super) fn caja(contenido: PanelElement<'_>, ancho: f32) -> PanelElement<'_> {
    container(contenido)
        .width(Length::Fixed(ancho))
        .style(|_theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_POPOVER.into(),
                width: 1.0,
                color: tema::borde(),
            },
            ..Default::default()
        })
        .into()
}

/// Una fila del menú. `señalada` va de 0 a 1: es el realce entrando.
///
/// El color se **interpola** en vez de conmutar. Con el hover de acento sólido
/// del plasmoide, encender y apagar de golpe se lee como un parpadeo al pasar
/// el ratón por el menú de arriba abajo; los 120 ms del token de hover lo
/// convierten en un rastro.
// 0.1.0
// modificado en la 0.2.0 -- icono opcional
pub(super) fn fila(
    etiqueta: &str,
    atajo: Option<&str>,
    icono: Option<&str>,
    señalada: f32,
) -> PanelElement<'static> {
    fila_con(etiqueta, atajo, icono.map(Delante::Icono), false, false, señalada)
}

/// Lo que va delante del texto de una fila.
// añadido en la 0.2.3
#[derive(Clone, Copy)]
pub(super) enum Delante<'a> {
    Icono(&'a str),
    /// Una casilla: la marca de «hecho» si está puesta, y el hueco si no, para
    /// que el texto no baile entre una fila marcada y otra sin marcar.
    Casilla(bool),
    /// La opción de un grupo excluyente: el círculo del popover del HIG §9,
    /// relleno con su punto cuando es la elegida.
    Opcion(bool),
}

/// La tinta de lo destructivo como **texto** sobre la tarjeta.
///
/// El rojo de estado del tema claro (`#FF3B30`) da 3,6:1 sobre blanco y el HIG
/// §2 pide 4,5 para texto accionable: se oscurece hacia la tinta hasta llegar.
/// En oscuro ya cumple y sale tal cual.
// añadido en la 0.2.3
pub(super) fn rojo_texto() -> iced_core::Color {
    let fondo = tema::card();
    (0..=10)
        .map(|n| tema::mezclar(tema::rojo(), tema::texto(), n as f32 * 0.05))
        .find(|c| tema::contraste(*c, fondo) >= 4.5)
        .unwrap_or_else(tema::texto)
}

/// Una fila completa del menú. `peligro` la tiñe de rojo y su realce es rojo
/// en vez de acento: azul es acción neutra (HIG §1) y lo que borra no lo es.
/// `chevron` dice que despliega un submenú; va con su icono y no con «›»,
/// que el HIG §10 no admite como pictograma.
// añadido en la 0.2.3
pub(super) fn fila_con(
    etiqueta: &str,
    atajo: Option<&str>,
    delante: Option<Delante<'_>>,
    peligro: bool,
    chevron: bool,
    señalada: f32,
) -> PanelElement<'static> {
    // El hover del plasmoide es acento **sólido** con el texto en blanco, no un
    // gris sutil: es lo que hace que se lea como un menú de sistema. El fondo
    // entra por alfa y no mezclándose con la tarjeta porque debajo hay un
    // popover translúcido: mezclar con `card()` dejaría un rectángulo opaco.
    let base = if peligro { tema::rojo() } else { tema::acento() };
    let fondo = tema::alfa(base, señalada);
    let sobre = tema::tinta_sobre(base);
    let reposo = if peligro { rojo_texto() } else { tema::texto() };
    let color = tema::mezclar(reposo, sobre, señalada);
    // Los iconos van en gris para que mande el texto; los de una fila roja,
    // rojos como ella.
    let reposo_icono = if peligro { reposo } else { tema::texto2() };
    let color_icono = tema::mezclar(reposo_icono, sobre, señalada);

    // El icono se tiñe del color de la fila para que siga al texto al pasar a
    // fondo de acento; si no, quedaría blanco sobre azul con el mismo tono y
    // desaparecería justo en la fila que se está mirando.
    let mut contenido = row![].align_y(iced_core::alignment::Vertical::Center);
    if let Some(delante) = delante {
        contenido = contenido.push(match delante {
            Delante::Icono(nombre) => match crate::icono::propio(nombre) {
                Some(ic) => crate::icono::ver_teñido_propio(&ic, ICONO, color_icono),
                // Un icono que falta no puede descolocar el texto de su fila
                // respecto a las demás: se deja el hueco.
                None => Space::new().width(Length::Fixed(ICONO)).into(),
            },
            Delante::Casilla(true) => match crate::icono::propio("comprobado") {
                Some(ic) => crate::icono::ver_teñido_propio(
                    &ic,
                    ICONO,
                    tema::mezclar(tema::acento(), sobre, señalada),
                ),
                None => Space::new().width(Length::Fixed(ICONO)).into(),
            },
            Delante::Casilla(false) => Space::new().width(Length::Fixed(ICONO)).into(),
            Delante::Opcion(elegida) => opcion(elegida, señalada),
        });
        contenido = contenido.push(Space::new().width(Length::Fixed(HUECO_ICONO)));
    }
    contenido = contenido.push(text(etiqueta.to_string()).size(tema::T_CUERPO).color(color));
    contenido = contenido.push(Space::new().width(Length::Fill));
    if let Some(atajo) = atajo {
        let color_atajo = tema::mezclar(tema::texto2(), sobre, señalada);
        contenido = contenido.push(teclas(atajo, color_atajo));
    }
    if chevron {
        contenido = contenido.push(match crate::icono::propio("chevron-derecha") {
            Some(ic) => crate::icono::ver_teñido_propio(&ic, CHEVRON, color_icono),
            None => Space::new().width(Length::Fixed(CHEVRON)).into(),
        });
    }

    container(
        container(contenido)
            .padding([0, RELLENO as u16])
            .height(Length::Fill)
            .center_y(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_ITEM_POPOVER.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .padding([0, MARGEN as u16])
    .height(Length::Fixed(FILA))
    .into()
}

/// Lado del chevron de submenú: un poco menor que el icono de la fila, como
/// en la maqueta, para que no compita con el texto.
pub(super) const CHEVRON: f32 = 14.0;

/// El círculo de una opción excluyente (HIG §9: borde 2, elegida rellena con
/// un punto de 6). Del mismo lado que un icono, para que el texto no se mueva.
// añadido en la 0.2.3
fn opcion(elegida: bool, señalada: f32) -> PanelElement<'static> {
    let sobre = tema::sobre_acento();
    let (relleno, borde, punto) = if elegida {
        (
            tema::mezclar(tema::acento(), sobre, señalada),
            tema::mezclar(tema::acento(), sobre, señalada),
            Some(tema::mezclar(sobre, tema::acento(), señalada)),
        )
    } else {
        (
            iced_core::Color::TRANSPARENT,
            // Gris secundario entero: con menos alfa el círculo vacío no llegaba
            // al 3:1 de control del HIG §2 sobre la tarjeta clara.
            tema::mezclar(tema::texto2(), sobre, señalada),
            None,
        )
    };
    container(match punto {
        Some(c) => container(Space::new())
            .width(Length::Fixed(6.0))
            .height(Length::Fixed(6.0))
            .style(move |_| container::Style {
                background: Some(c.into()),
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
        None => PanelElement::from(Space::new()),
    })
    .width(Length::Fixed(ICONO))
    .height(Length::Fixed(ICONO))
    .center_x(Length::Fixed(ICONO))
    .center_y(Length::Fixed(ICONO))
    .style(move |_| container::Style {
        background: Some(relleno.into()),
        border: Border {
            radius: (ICONO / 2.0).into(),
            width: 2.0,
            color: borde,
        },
        ..Default::default()
    })
    .into()
}

/// Cómo se lee cada tecla en el menú: con palabras, no con los símbolos de
/// Apple (⌘ ⌃ ⌥). En BookOS la tecla del sistema es Súper y ese es el nombre
/// que lleva en el teclado de quien lo usa; los símbolos solo los entiende
/// quien viene de macOS. Lo que no está aquí —letras— se escribe tal cual.
// modificado en la 0.2.0 -- palabras en teclas pequeñas, sin símbolos de Mac
fn nombre_tecla(tecla: &str) -> &str {
    match tecla {
        "Meta" => "Súper",
        otra => otra,
    }
}

/// Lo que ocupa una tecla dibujada: la palabra y el relleno de la cápsula.
fn ancho_tecla(tecla: &str) -> f32 {
    crate::widget::ancho_de(nombre_tecla(tecla), T_ATAJO) + RELLENO_TECLA * 2.0
}

/// Lo que ocupa un atajo dibujado con [`teclas`].
// añadido en la 0.2.0
pub(super) fn ancho_atajo(atajo: &str) -> f32 {
    let partes = atajo.split('+');
    let huecos = (partes.clone().count() - 1) as f32 * HUECO_TECLA;
    partes.map(ancho_tecla).sum::<f32>() + huecos
}

/// Un atajo como fila de teclas pequeñas, una cápsula por tecla y sin «+».
/// La cápsula toma el color del texto del atajo con poco alfa, así que sigue a
/// la fila cuando el hover la pasa a fondo de acento.
// modificado en la 0.2.0 -- cápsulas con palabras
fn teclas(atajo: &str, color: iced_core::Color) -> PanelElement<'static> {
    let mut fila = row![]
        .spacing(HUECO_TECLA)
        .align_y(iced_core::alignment::Vertical::Center);
    for tecla in atajo.split('+') {
        fila = fila.push(
            container(
                text(nombre_tecla(tecla).to_string())
                    .size(T_ATAJO)
                    .color(color),
            )
            .padding([1, RELLENO_TECLA as u16])
            .style(move |_theme| container::Style {
                background: Some(tema::alfa(color, 0.16).into()),
                border: Border {
                    radius: 5.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
    }
    fila.into()
}

// 0.1.0
pub(super) fn divisor() -> PanelElement<'static> {
    container(
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            }),
    )
    .padding([5, RELLENO as u16 + MARGEN as u16])
    .height(Length::Fixed(DIVISOR))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TeclaPulsada;

    #[test]
    fn teclado_navega_y_ejecuta_la_entrada_senalada() {
        let mut m = Menu::new();
        assert!(matches!(m.tecla(TeclaPulsada::Intro), Tecla::Ignorada));
        assert!(matches!(m.tecla(TeclaPulsada::Abajo), Tecla::Consumida));
        assert!(matches!(
            m.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::Acerca)
        ));
        assert!(!m.accesible().is_empty());
    }
}
