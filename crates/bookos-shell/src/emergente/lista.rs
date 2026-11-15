//! Las piezas que comparten las tarjetas de conectividad: Wi-Fi y Bluetooth.
//!
//! Las dos son la misma tarjeta del diseño (`Applets` en Figma): título con un
//! interruptor a la derecha y, debajo, un grupo gris con las filas en baldosas
//! —icono, nombre, estado y un número a la derecha— y un pie con dos botones.
//! Sin esto habría dos copias de la misma geometría que se irían separando a la
//! primera corrección.
//!
//! Las medidas son las del lienzo «Widgets de BookOS»: tarjeta de 336 con
//! margen de 16, cabecera de 32, grupo con 6 de relleno y baldosas de 48.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

use super::control::{self, GRUPO};

/// Ancho de la tarjeta.
pub const ANCHO: f32 = 336.0;
pub const MARGEN: f32 = 16.0;
/// Ancho de una baldosa: la tarjeta menos su margen y el relleno del grupo.
pub const BALDOSA: f32 = ANCHO - MARGEN * 2.0 - GRUPO * 2.0;
/// Alto de una fila de la lista.
pub const FILA: f32 = 48.0;
/// Hueco entre filas.
pub const HUECO_FILA: f32 = 6.0;
/// Lado del icono de una fila.
pub const ICONO: f32 = 22.0;
/// El disco que lleva el icono: el de los conmutadores del centro de control
/// menos 4, que la fila es de 48 y no de 56.
pub const DISCO: f32 = 36.0;
/// Alto de la cabecera, con el título y el interruptor.
pub const CABECERA: f32 = 32.0;
/// Aire entre la cabecera y el grupo.
pub const BAJO_CABECERA: f32 = 12.0;
/// Alto de los botones del pie.
pub const PIE_BOTON: f32 = 36.0;
/// Lo que hay entre la última fila y los botones: hueco, divisor con su aire de
/// 2 px por lado, y otro hueco.
pub const PIE_AIRE: f32 = HUECO_FILA + 2.0 + 0.5 + 2.0 + HUECO_FILA;
/// Alto del bloque entero del pie.
pub const PIE: f32 = PIE_AIRE + PIE_BOTON;
/// Medidas del interruptor: pista de 46×28 y bolita de 22 con 3 de margen,
/// las del sistema de diseño.
pub const INTERRUPTOR_ANCHO: f32 = 46.0;
pub const INTERRUPTOR_ALTO: f32 = 28.0;

/// Lo mismo con el alto de la ventana y lo que se puede subir dados por quien
/// llama: una lista con cabeceras de sección o filas desplegadas no tiene filas
/// de alto uniforme, y `n` no basta para saber cuánto mide.
// añadido en la 0.2.1
pub fn ventana_alto<'a>(
    filas: PanelElement<'a>,
    alto: f32,
    maximo: f32,
    desplazamiento: f32,
) -> PanelElement<'a> {
    ventana_ancha(filas, BALDOSA, alto, maximo, desplazamiento)
}

/// Lo mismo con el ancho también dado: las notificaciones no van dentro de un
/// grupo gris y sus tarjetas son más anchas que una baldosa.
// añadido en la 0.2.1
pub fn ventana_ancha<'a>(
    filas: PanelElement<'a>,
    ancho: f32,
    alto: f32,
    maximo: f32,
    desplazamiento: f32,
) -> PanelElement<'a> {
    let lista = crate::recorte::Recorte::new(filas, ancho, alto, desplazamiento);
    if maximo <= 0.0 {
        return lista.into();
    }
    let tirador = (alto * alto / (alto + maximo)).max(TIRADOR_MINIMO);
    let arriba = (alto - tirador) * (desplazamiento / maximo).clamp(0.0, 1.0);
    let pista = column![
        Space::new().height(Length::Fixed(arriba)),
        container(Space::new())
            .width(Length::Fixed(TIRADOR_ANCHO))
            .height(Length::Fixed(tirador))
            .style(|_| container::Style {
                background: Some(tema::alfa(tema::texto(), 0.35).into()),
                border: Border {
                    radius: (TIRADOR_ANCHO / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    ];
    iced_widget::stack![
        lista,
        container(pista)
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .padding(iced_core::Padding {
                right: 2.0,
                ..Default::default()
            })
            .align_right(Length::Fill),
    ]
    .into()
}

const TIRADOR_ANCHO: f32 = 3.0;
const TIRADOR_MINIMO: f32 = 24.0;

/// Una entrada de la lista: una red o un dispositivo.
pub struct Entrada {
    pub nombre: String,
    /// Debajo del nombre, en pequeño: «Conectado», «No conectado»…
    pub estado: String,
    /// El número de la derecha: la señal de una red o la batería de unos
    /// auriculares. Sin unidad, que la pone quien la dibuja.
    pub derecha: Option<String>,
    pub icono: Option<Icono>,
    /// Se dibuja como seleccionada.
    pub activa: bool,
    /// Un candado a la derecha, en lugar del número: la red pide clave.
    // añadido en la 0.2.1
    pub candado: bool,
}

/// Una pieza de una lista con filas de distinto alto —rótulos de sección,
/// filas, lo desplegado bajo una fila—, con su `y` dentro de la lista. La
/// geometría de la ventana, del ratón y del teclado sale de aquí y no de
/// `n × FILA`, que solo vale cuando todas miden lo mismo.
// añadido en la 0.2.1
#[derive(Clone, Copy, Debug)]
pub struct Colocada<F> {
    pub fila: F,
    pub y: f32,
    pub alto: f32,
}

/// Añade una pieza a la lista en la `y` actual y baja `y` hasta la siguiente,
/// con [`HUECO_FILA`] de aire.
// añadido en la 0.2.1
pub fn poner<F>(v: &mut Vec<Colocada<F>>, y: &mut f32, fila: F, alto: f32) {
    v.push(Colocada { fila, y: *y, alto });
    *y += alto + HUECO_FILA;
}

/// El alto de la ventana de una lista de `total` píxeles: lo que mide hasta
/// `tope`, o una fila para el aviso de «vacío».
// añadido en la 0.2.1
pub fn ventana_de(total: f32, tope: f32) -> f32 {
    if total <= 0.0 { FILA } else { total.min(tope) }
}

/// El interruptor de la cabecera. `encendido` va de 0 a 1: es el recorrido de
/// la bolita, no un booleano.
///
/// El sistema de diseño le da 250 ms con muelle (§2.7, «conmutador»), que es la
/// duración de [`crate::tema::D_MODAL`]: el estado del interruptor **es** la
/// decisión que se acaba de tomar y se ve mejor que ninguna otra cosa de la
/// tarjeta. El que lo use guarda una [`crate::tema::Transicion`] y le pasa aquí
/// su valor.
// 0.1.0
pub fn interruptor<'a>(encendido: f32) -> PanelElement<'a> {
    let lado = INTERRUPTOR_ALTO - 6.0;
    let bolita = container(Space::new())
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border {
                radius: (lado / 2.0).into(),
                ..Default::default()
            },
            shadow: iced_core::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
                offset: iced_core::Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            },
            ..Default::default()
        });
    // La bolita se coloca con un hueco a un lado o al otro en vez de con una
    // posición absoluta: evita una capa `stack` solo para mover 18 px. Animada,
    // el hueco es el recorrido entero por la fracción, y con el muelle se pasa
    // un poco del extremo — por eso se recorta a cero: un hueco negativo no lo
    // acepta el layout.
    let recorrido = (INTERRUPTOR_ANCHO - INTERRUPTOR_ALTO) * encendido;
    let dentro: PanelElement<'a> = row![
        Space::new().width(Length::Fixed(recorrido.max(0.0))),
        bolita
    ]
    .into();
    container(dentro)
        .width(Length::Fixed(INTERRUPTOR_ANCHO))
        .height(Length::Fixed(INTERRUPTOR_ALTO))
        .padding(3)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            // Del `--toff` opaco al acento con la bolita. El recorte a [0,1] es
            // por el muelle otra vez: pasarse de 1 en un canal de color no es
            // un rebote, es un color inventado.
            background: Some(
                tema::mezclar(
                    tema::control_apagado(),
                    tema::acento(),
                    encendido.clamp(0.0, 1.0),
                )
                .into(),
            ),
            border: Border {
                radius: (INTERRUPTOR_ALTO / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// La cabecera: título a la izquierda, interruptor a la derecha.
///
/// Los 4 px de relleno lateral dejan el título a 20 del borde de la tarjeta,
/// que es el padding de una fila en el sistema de diseño, mientras el grupo de
/// debajo se queda a 16.
/// ¿Cae el punto en el interruptor de la cabecera? En coordenadas de la
/// tarjeta, con las mismas medidas que [`cabecera`]: pegado a la derecha, con
/// los 4 de relleno de la cabecera y el margen de la tarjeta.
///
/// Toda la altura de la cabecera y 8 más a cada lado: 46×28 es poco para
/// acertar con el touchpad, y a su izquierda solo está el título.
// 0.1.0
pub fn interruptor_en(x: f32, y: f32) -> bool {
    let derecha = ANCHO - MARGEN - 4.0;
    let izquierda = derecha - INTERRUPTOR_ANCHO;
    (izquierda - 8.0..=derecha + 8.0).contains(&x) && (MARGEN..=MARGEN + CABECERA).contains(&y)
}

/// Lado de la flecha de volver de la cabecera, y lo que se aparta el título.
const VOLVER: f32 = 18.0;
const TRAS_VOLVER: f32 = 6.0;

/// ¿Cae el punto en la flecha de volver? Toda la altura de la cabecera y hasta
/// el principio del título, más algo de aire: es un blanco de 18 px.
// 0.1.0
pub fn volver_en(x: f32, y: f32) -> bool {
    (MARGEN - 4.0..=MARGEN + 4.0 + VOLVER + TRAS_VOLVER + 8.0).contains(&x)
        && (MARGEN..=MARGEN + CABECERA).contains(&y)
}

/// El título de la tarjeta y su interruptor. Con `volver`, una flecha delante
/// del título: la tarjeta está dentro de la Estación de Control y se vuelve a
/// ella, como el applet de Plasma dentro de la bandeja del sistema.
// 0.1.0
pub fn cabecera<'a>(titulo: &'a str, encendido: Option<f32>, volver: bool) -> PanelElement<'a> {
    let mut fila = row![].align_y(Vertical::Center);
    if volver {
        let flecha: PanelElement<'a> = match icono::propio("chevron-izquierda") {
            Some(ic) => icono::ver_teñido_propio(&ic, VOLVER, tema::texto()),
            None => Space::new().width(Length::Fixed(VOLVER)).into(),
        };
        fila = fila
            .push(flecha)
            .push(Space::new().width(Length::Fixed(TRAS_VOLVER)));
    }
    fila = fila.push(control::titulo(titulo));
    if let Some(encendido) = encendido {
        fila = fila
            .push(Space::new().width(Length::Fill))
            .push(interruptor(encendido));
    }
    container(fila)
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(CABECERA))
        .padding([0, 4])
        .center_y(Length::Fixed(CABECERA))
        .into()
}

/// El grupo gris que envuelve la lista y su pie.
// 0.1.0
// modified 0.2.1 -- radio de las tarjetas del centro de control
pub fn grupo<'a>(contenido: PanelElement<'a>) -> PanelElement<'a> {
    // El radio de 13 de `control::grupo` es de los menús; dentro de la estación
    // los grupos van a `R_TARJETA` y la lista se abre ahí.
    container(contenido)
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .padding(GRUPO)
        .style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Lo que le queda al nombre de una fila una vez descontados el relleno, el
/// icono, su hueco y el número de la derecha.
///
/// Sin esto un SSID largo —los de las operadoras traen doce dígitos— empuja al
/// número fuera de la tarjeta y la fila se queda sin señal ni batería. Medido
/// con «2qxm79NAQsjDVNVj68nTTLF2CsZ9UJ8m»: ocupaba los 293 px enteros.
const ANCHO_NOMBRE: f32 = BALDOSA - 12.0 - DISCO - 12.0 - 44.0;

/// Recorta un texto al ancho disponible y le pone puntos suspensivos.
///
/// La medida es la aproximación de [`crate::widget::ancho_texto`], que es un
/// ancho medio por carácter: el shell no tiene acceso al `Paragraph` de iced
/// antes de construirlo, y un carácter de más o de menos aquí no rompe nada —el
/// hueco reservado son 44 px.
// 0.1.0
pub fn recortar(texto: &str, ancho: f32, tamaño: f32) -> String {
    let por_letra = crate::widget::ancho_texto(1) * tamaño / crate::tema::T_CUERPO;
    let caben = (ancho / por_letra).floor() as usize;
    if texto.chars().count() <= caben {
        return texto.to_string();
    }
    // Uno menos para dejarle sitio a los puntos, que ocupan como una letra.
    texto
        .chars()
        .take(caben.saturating_sub(1))
        .collect::<String>()
        + "…"
}

/// Una fila de la lista. `señalada` va de 0 a 1: el hover entrando.
// 0.1.0
// modified 0.2.1 -- el lenguaje del centro de control: disco con el glifo y fila sin baldosa
pub fn fila<'a>(entrada: &'a Entrada, señalada: f32) -> PanelElement<'a> {
    // Lo activo se marca como un conmutador encendido de la estación (disco
    // blanco con el glifo en acento) y no con un relleno de acento: la fila
    // tintada con filo era lo único naranja de la tarjeta y se leía como un
    // aviso. Las demás filas no llevan baldosa; solo el hover las levanta.
    let (disco, glifo, estado_color) = if entrada.activa {
        let (d, g) = control::encendido();
        (d, g, tema::acento())
    } else {
        (tema::elevado(), tema::texto(), tema::TEXTO2)
    };
    // Opaco a propósito: un relleno translúcido en tiny-skia cuesta ~17× más
    // y la fila se repinta en cada paso del hover.
    let fondo = tema::mezclar(tema::superficie_en_tarjeta(), tema::elevado(), señalada);
    let mut textos = column![
        text(recortar(&entrada.nombre, ANCHO_NOMBRE, 14.0))
            .size(14.0)
            .font(control::peso(if entrada.activa {
                Weight::Semibold
            } else {
                Weight::Medium
            }))
            .color(tema::texto())
    ];
    if !entrada.estado.is_empty() {
        textos = textos.push(
            text(recortar(&entrada.estado, ANCHO_NOMBRE, 11.0))
                .size(11.0)
                .color(estado_color),
        );
    }
    let dentro: PanelElement<'a> = match &entrada.icono {
        Some(ic) => icono::ver_teñido(ic, ICONO * 0.82, Some(glifo)),
        None => Space::new().into(),
    };
    let dibujo = container(dentro)
        .width(Length::Fixed(DISCO))
        .height(Length::Fixed(DISCO))
        .center_x(Length::Fixed(DISCO))
        .center_y(Length::Fixed(DISCO))
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(disco.into()),
            border: Border {
                radius: (DISCO / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    let mut contenido =
        row![dibujo, Space::new().width(Length::Fixed(12.0)), textos].align_y(Vertical::Center);
    if let Some(derecha) = &entrada.derecha {
        contenido = contenido
            .push(Space::new().width(Length::Fill))
            .push(text(derecha.as_str()).size(12.0).color(tema::TEXTO2));
    } else if entrada.candado
        && let Some(ic) = icono::propio_solido("bloquear")
    {
        contenido = contenido
            .push(Space::new().width(Length::Fill))
            .push(icono::ver_teñido_propio(&ic, 13.0, tema::TEXTO2));
    }
    container(contenido)
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(FILA))
        .center_y(Length::Fixed(FILA))
        // 6 a la izquierda para que el disco quede a la misma distancia del
        // borde que del techo; 12 a la derecha para no tocar el tirador.
        .padding(iced_core::Padding {
            left: 6.0,
            right: 12.0,
            ..Default::default()
        })
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Alto del rótulo de una sección («Mis dispositivos»).
// añadido en la 0.2.1
pub const SECCION: f32 = 26.0;

/// El rótulo de una sección de la lista. Va a la altura del texto de las filas
/// (6 de relleno, como el disco) para que el rótulo y los nombres queden en
/// la misma vertical. `buscando` pone al lado un punto que late: dice que la
/// lista aún se está llenando, que si no una sección vacía parece un fallo.
// añadido en la 0.2.1
pub fn seccion<'a>(titulo: &'a str, buscando: bool) -> PanelElement<'a> {
    let mut fila = row![
        text(titulo)
            .size(12.0)
            .font(control::peso(Weight::Semibold))
            .color(tema::TEXTO2)
    ]
    .spacing(8)
    .align_y(Vertical::Center);
    if buscando {
        fila = fila.push(text("buscando…").size(11.0).color(tema::TEXTO2));
    }
    container(fila)
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(SECCION))
        .padding(iced_core::Padding {
            left: 6.0,
            ..Default::default()
        })
        .center_y(Length::Fixed(SECCION))
        .into()
}

/// Un renglón cuando no hay nada que listar.
// 0.1.0
pub fn vacia<'a>(texto_: &str) -> PanelElement<'a> {
    container(text(texto_.to_string()).size(13.0).color(tema::TEXTO2))
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(FILA))
        .center_x(Length::Fixed(BALDOSA))
        .center_y(Length::Fixed(FILA))
        .into()
}

/// El pie: un solo botón a Ajustes. Eran dos («Detalles» y «Configuración»)
/// que abrían la misma página.
// 0.1.0
// modified 0.2.1 -- un botón a lo ancho
pub fn pie<'a>(etiqueta: &'a str, realce: &tema::Realce) -> PanelElement<'a> {
    let realzado = realce.intensidad(0);
    column![
        Space::new().height(Length::Fixed(PIE_AIRE)),
        container(
            text(etiqueta)
                .size(tema::T_CUERPO)
                .font(control::peso(Weight::Semibold))
                .color(tema::texto()),
        )
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(PIE_BOTON))
        .center_x(Length::Fixed(BALDOSA))
        .center_y(Length::Fixed(PIE_BOTON))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(control::baldosa(realzado).into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    ]
    .into()
}

/// ¿Cae el punto en el botón del pie, si el pie empieza en `y0`?
// 0.1.0
// modified 0.2.1 -- un solo botón
pub fn pie_en(x: f32, y: f32, y0: f32) -> bool {
    (y0..=y0 + PIE_BOTON).contains(&y) && x > MARGEN + GRUPO && x < ANCHO - MARGEN - GRUPO
}

#[cfg(test)]
mod tests {

    use super::*;

    /// El centro del interruptor se pulsa y el título no: los dos comparten la
    /// cabecera, y confundirlos apagaría el Bluetooth al querer leer el título.
    #[test]
    fn el_interruptor_de_la_cabecera_responde_donde_se_dibuja() {
        let centro_x = ANCHO - MARGEN - 4.0 - INTERRUPTOR_ANCHO / 2.0;
        let centro_y = MARGEN + CABECERA / 2.0;
        assert!(interruptor_en(centro_x, centro_y));
        assert!(
            !interruptor_en(MARGEN + 20.0, centro_y),
            "el título no es el interruptor"
        );
        assert!(
            !interruptor_en(centro_x, MARGEN + CABECERA + 20.0),
            "debajo ya es la lista"
        );
    }

    /// Un SSID largo se recorta con puntos en vez de empujar al número de la
    /// derecha fuera de la tarjeta.
    #[test]
    fn un_nombre_largo_se_recorta() {
        let largo = "2qxm79NAQsjDVNVj68nTTLF2CsZ9UJ8m";
        let corto = recortar(largo, ANCHO_NOMBRE, 14.0);
        assert!(corto.ends_with('…'), "{corto}");
        assert!(corto.chars().count() < largo.chars().count());
        assert_eq!(
            recortar("Wifi", ANCHO_NOMBRE, 14.0),
            "Wifi",
            "lo que cabe no se toca"
        );
    }

}
