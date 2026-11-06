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

use super::{Ancla, Tecla};

/// Lo que el menú puede crecer para que quepa un nombre largo en «Cerrar
/// sesión de …». Más allá se recorta: un menú de medio panel tapa el escritorio
/// que se quería mirar.
const ANCHO_MIN: f32 = 200.0;
const ANCHO_MAX: f32 = 300.0;
/// 24 y no 26: con 26 el menú del logo se leía como una lista de botones.
/// 24 sigue siendo alcanzable con el dedo y queda a la densidad del menú de
/// macOS (22 pt a 13 pt de letra), que es la referencia.
pub(super) const FILA: f32 = 24.0;
/// Alto del bloque separador, con su línea de 1 px en medio.
pub(super) const DIVISOR: f32 = 9.0;
/// Margen lateral de las filas dentro del menú.
pub(super) const MARGEN: f32 = 5.0;
/// Márgenes de arriba y de abajo del menú entero: iguales al lateral, para que
/// el realce de la primera y la última fila quede concéntrico con la esquina.
const BORDE_SUP: f32 = MARGEN;
const BORDE_INF: f32 = MARGEN;
/// Relleno del texto dentro de la fila, a cada lado.
const RELLENO: f32 = 10.0;
/// Aire mínimo entre la etiqueta y su atajo.
const HUECO_ATAJO: f32 = 24.0;
/// Letra de los atajos, y lado de los símbolos de modificador que la
/// acompañan. El símbolo va un punto por encima de la letra: dibujado a la
/// misma medida, el trazo de 1,75 sobre 24 lo dejaba más fino y bajo que la
/// «L» de al lado.
const T_ATAJO: f32 = tema::T_PEQUENO - 1.0;
const SIMBOLO: f32 = T_ATAJO + 1.0;
/// Entre una tecla y la siguiente del mismo atajo: pegadas, como en macOS,
/// pero sin que el ⌥ toque al ⌃.
const HUECO_TECLA: f32 = 1.0;

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
// modificado en la 0.2.0 -- sin iconos y «Cerrar sesión de <nombre>…»
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
        item("Acerca de este PC", None, || Accion::Acerca),
        Divisor,
        // Antes de «Preferencias del sistema…», que abre otro programa: esta
        // es del propio escritorio y se aplica al pulsarla.
        item("Apariencia…", None, || Accion::Emergente("apariencia")),
        item("Preferencias del sistema…", None, || {
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
        item("Dormir", None, || {
            Accion::Lanzar("systemctl suspend".into())
        }),
    ];
    // Debajo de Dormir, y solo si logind dice que se puede: sin swap en disco
    // el botón fallaría sin más.
    if crate::confirmacion::hibernar_disponible() {
        entradas.push(item("Hibernar", None, || {
            Accion::Lanzar("systemctl hibernate".into())
        }));
    }
    let login = std::env::var("USER").unwrap_or_default();
    let nombre = crate::bloqueo::nombre_completo(&login).unwrap_or(login);
    entradas.extend([
        item("Reiniciar…", None, || {
            Accion::Lanzar("systemctl reboot".into())
        }),
        item("Apagar…", None, || {
            Accion::Lanzar("systemctl poweroff".into())
        }),
        Divisor,
        item("Pantalla de bloqueo", Some("Meta+L"), || Accion::Bloquear),
        Item {
            // Con el nombre, como en macOS: en un equipo con varias cuentas
            // dice cuál se va a cerrar. Sin `USER` queda la frase sola.
            etiqueta: if nombre.is_empty() {
                "Cerrar sesión…".into()
            } else {
                format!("Cerrar sesión de {nombre}…")
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
            T::Abajo => self.mover(1),
            T::Arriba => self.mover(-1),
            // Sin selección, Intro no hace nada: mejor eso que activar la
            // primera entrada, que aquí sería "Acerca de este PC" y en otro
            // menú podría ser "Apagar".
            T::Intro => Tecla::Ignorada,
            _ => Tecla::Ignorada,
        }
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
/// El radio es el de botón pequeño (10) y no el de popover (18): un menú es
/// una lista estrecha, y con 18 sobre 200 px de ancho las esquinas se comían
/// la primera y la última fila y se leía como una burbuja. 10 es lo que usa el
/// menú de macOS, y con el margen de 5 deja el realce de radio 6 concéntrico.
// 0.2.0
pub(super) fn caja(contenido: PanelElement<'_>, ancho: f32) -> PanelElement<'_> {
    container(contenido)
        .width(Length::Fixed(ancho))
        .style(|_theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
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
    // El hover del plasmoide es acento **sólido** con el texto en blanco, no un
    // gris sutil: es lo que hace que se lea como un menú de sistema. El fondo
    // entra por alfa y no mezclándose con la tarjeta porque debajo hay un
    // popover translúcido: mezclar con `card()` dejaría un rectángulo opaco.
    let fondo = tema::alfa(tema::acento(), señalada);
    let color = tema::mezclar(tema::texto(), tema::sobre_acento(), señalada);

    // El icono se tiñe del color de la fila para que siga al texto al pasar a
    // fondo de acento; si no, quedaría blanco sobre azul con el mismo tono y
    // desaparecería justo en la fila que se está mirando.
    let mut contenido = row![].align_y(iced_core::alignment::Vertical::Center);
    if let Some(icono) = icono {
        contenido = contenido.push(match crate::icono::propio(icono) {
            Some(ic) => crate::icono::ver_teñido_propio(&ic, ICONO, color),
            // Un icono que falta no puede descolocar el texto de su fila
            // respecto a las demás: se deja el hueco.
            None => Space::new().width(Length::Fixed(ICONO)).into(),
        });
        contenido = contenido.push(Space::new().width(Length::Fixed(HUECO_ICONO)));
    }
    contenido = contenido.push(text(etiqueta.to_string()).size(tema::T_CUERPO).color(color));
    contenido = contenido.push(Space::new().width(Length::Fill));
    if let Some(atajo) = atajo {
        let color_atajo = tema::mezclar(tema::TEXTO2, tema::sobre_acento(), señalada);
        contenido = contenido.push(teclas(atajo, color_atajo));
    }

    container(
        container(contenido)
            .padding([0, RELLENO as u16])
            .height(Length::Fill)
            .center_y(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_CHIP.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .padding([0, MARGEN as u16])
    .height(Length::Fixed(FILA))
    .into()
}

/// El icono que sustituye a un modificador escrito, como los de macOS:
/// «Meta+L» se lee ⌘L. Meta va a ⌘ porque es la tecla del sistema, la que en
/// BookOS hace lo que en macOS hace Comando. Lo que no está aquí —letras, «›»
/// del dock— se queda en texto.
// añadido en la 0.2.0
fn simbolo(tecla: &str) -> Option<&'static str> {
    match tecla {
        "Meta" => Some("tecla-cmd"),
        "Ctrl" => Some("tecla-ctrl"),
        "Alt" => Some("tecla-alt"),
        "Mayús" => Some("mayus"),
        "Supr" => Some("tecla-supr"),
        _ => None,
    }
}

/// Lo que ocupa un atajo dibujado con [`teclas`].
// añadido en la 0.2.0
fn ancho_atajo(atajo: &str) -> f32 {
    let partes = atajo.split('+');
    let huecos = (partes.clone().count() - 1) as f32 * HUECO_TECLA;
    partes
        .map(|t| match simbolo(t) {
            Some(_) => SIMBOLO,
            None => crate::widget::ancho_de(t, T_ATAJO),
        })
        .sum::<f32>()
        + huecos
}

/// Un atajo como fila de teclas: símbolo para cada modificador y la tecla
/// final en texto, sin los «+».
// añadido en la 0.2.0
fn teclas(atajo: &str, color: iced_core::Color) -> PanelElement<'static> {
    let mut fila = row![]
        .spacing(HUECO_TECLA)
        .align_y(iced_core::alignment::Vertical::Center);
    for tecla in atajo.split('+') {
        fila = fila.push(match simbolo(tecla).and_then(crate::icono::propio) {
            Some(ic) => crate::icono::ver_teñido_propio(&ic, SIMBOLO, color),
            None => text(tecla.to_string()).size(T_ATAJO).color(color).into(),
        });
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
    .padding([4, RELLENO as u16 + MARGEN as u16])
    .height(Length::Fixed(DIVISOR))
    .into()
}
