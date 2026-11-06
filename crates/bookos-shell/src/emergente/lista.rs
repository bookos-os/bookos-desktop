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

/// Cuántas filas se ven a la vez. Con más entradas la lista se desplaza: la
/// tarjeta no crece, que a más de cuatro redes llegaría al dock.
// añadido en la 0.2.0
pub const VISIBLES: usize = 4;

/// Alto de la ventana por la que se ve la lista: de una fila (el aviso de
/// «vacío») a [`VISIBLES`].
// añadido en la 0.2.0
pub fn alto_ventana(n: usize) -> f32 {
    let v = n.clamp(1, VISIBLES) as f32;
    v * FILA + (v - 1.0) * HUECO_FILA
}

/// Lo máximo que se puede subir la lista: lo que sobra de `n` filas sobre la
/// ventana.
// añadido en la 0.2.0
pub fn maximo_desplazamiento(n: usize) -> f32 {
    let n = n.max(1) as f32;
    (n * FILA + (n - 1.0) * HUECO_FILA - alto_ventana(n as usize)).max(0.0)
}

/// Aplica un desplazamiento de rueda o touchpad. `true` si la lista se movió.
/// `dy` positivo es hacia abajo, como en el resto de emergentes.
// añadido en la 0.2.0
pub fn desplazar(actual: &mut f32, dy: f32, n: usize) -> bool {
    let nuevo = (*actual + dy).clamp(0.0, maximo_desplazamiento(n));
    let cambio = (nuevo - *actual).abs() > f32::EPSILON;
    *actual = nuevo;
    cambio
}

/// Mueve la fila señalada con el teclado (`paso` ±1) y desplaza la lista lo
/// justo para que se vea entera. Sin cíclico: al llegar al final se queda, que
/// en una lista de redes saltar de la última a la primera desorienta. `true`
/// si cambió algo que haya que repintar.
// añadido en la 0.2.0
pub fn mover_seleccion(
    señalada: &mut tema::Realce,
    desplazamiento: &mut f32,
    n: usize,
    paso: i32,
) -> bool {
    if n == 0 {
        return false;
    }
    let nueva = match señalada.actual() {
        None if paso > 0 => 0,
        None => n - 1,
        Some(a) => (a as i32 + paso).clamp(0, n as i32 - 1) as usize,
    };
    let cambio = señalada.señalar(Some(nueva));
    let arriba = nueva as f32 * (FILA + HUECO_FILA);
    let alto = alto_ventana(n);
    if arriba < *desplazamiento {
        *desplazamiento = arriba;
    } else if arriba + FILA > *desplazamiento + alto {
        *desplazamiento = arriba + FILA - alto;
    }
    *desplazamiento = desplazamiento.clamp(0.0, maximo_desplazamiento(n));
    cambio
}

/// Las filas dentro de su ventana recortada. Recorte de capa y no
/// `.clip(true)`: ver `recorte.rs`.
///
/// Con más filas de las que caben lleva a la derecha un tirador fino, que
/// dice que hay más debajo y cuánto: sin él una lista cortada a media fila
/// solo se descubría al mover la rueda. No es agarrable; es un indicador.
// añadido en la 0.2.0
pub fn ventana<'a>(filas: PanelElement<'a>, n: usize, desplazamiento: f32) -> PanelElement<'a> {
    let alto = alto_ventana(n);
    let lista = crate::recorte::Recorte::new(filas, BALDOSA, alto, desplazamiento);
    let maximo = maximo_desplazamiento(n);
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
            .width(Length::Fixed(BALDOSA))
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
pub fn grupo<'a>(contenido: PanelElement<'a>) -> PanelElement<'a> {
    control::grupo(contenido, ANCHO - MARGEN * 2.0, GRUPO)
}

/// Lo que le queda al nombre de una fila una vez descontados el relleno, el
/// icono, su hueco y el número de la derecha.
///
/// Sin esto un SSID largo —los de las operadoras traen doce dígitos— empuja al
/// número fuera de la tarjeta y la fila se queda sin señal ni batería. Medido
/// con «2qxm79NAQsjDVNVj68nTTLF2CsZ9UJ8m»: ocupaba los 293 px enteros.
const ANCHO_NOMBRE: f32 = BALDOSA - 24.0 - ICONO - 12.0 - 44.0;

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
pub fn fila<'a>(entrada: &'a Entrada, señalada: f32) -> PanelElement<'a> {
    // La conectada va con el acento al 10 % y un filo de acento, con el texto
    // en acento: la referencia la marcaba con el filo, y el relleno tenue es lo
    // que el sistema de diseño pide para lo seleccionado en una lista.
    let (fondo, color, borde, estado_color) = if entrada.activa {
        (
            control::seleccionada(señalada),
            tema::acento(),
            tema::alfa(tema::acento(), 0.45),
            tema::acento(),
        )
    } else {
        (
            control::baldosa(señalada),
            tema::texto(),
            Color::TRANSPARENT,
            tema::TEXTO2,
        )
    };
    let mut textos = column![
        text(recortar(&entrada.nombre, ANCHO_NOMBRE, 14.0))
            .size(14.0)
            .font(control::peso(Weight::Medium))
            .color(color)
    ];
    if !entrada.estado.is_empty() {
        textos = textos.push(
            text(recortar(&entrada.estado, ANCHO_NOMBRE, 11.0))
                .size(11.0)
                .color(estado_color),
        );
    }
    let dibujo: PanelElement<'a> = match &entrada.icono {
        Some(ic) => icono::ver_teñido(
            ic,
            ICONO,
            Some(if entrada.activa {
                tema::acento()
            } else {
                tema::TEXTO2
            }),
        ),
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let mut contenido =
        row![dibujo, Space::new().width(Length::Fixed(12.0)), textos].align_y(Vertical::Center);
    if let Some(derecha) = &entrada.derecha {
        contenido = contenido
            .push(Space::new().width(Length::Fill))
            .push(text(derecha.as_str()).size(13.0).color(estado_color));
    }
    container(contenido)
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(FILA))
        .center_y(Length::Fixed(FILA))
        .padding([0, 12])
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                width: 1.0,
                color: borde,
            },
            ..Default::default()
        })
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

/// El pie con dos botones, dentro del grupo. `realce` señala el 0 o el 1.
// 0.1.0
pub fn pie<'a>(izquierda: &'a str, derecha: &'a str, realce: &tema::Realce) -> PanelElement<'a> {
    let ancho = (BALDOSA - HUECO_FILA) / 2.0;
    let boton = |etiqueta: &'a str, realzado: f32| -> PanelElement<'a> {
        container(
            text(etiqueta)
                .size(tema::T_CUERPO)
                .font(control::peso(Weight::Semibold))
                .color(tema::texto()),
        )
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(PIE_BOTON))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(PIE_BOTON))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(control::baldosa(realzado).into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };
    column![
        Space::new().height(Length::Fixed(HUECO_FILA + 2.0)),
        container(control::divisor(BALDOSA - 12.0)).padding([0, 6]),
        Space::new().height(Length::Fixed(2.0 + HUECO_FILA)),
        row![
            boton(izquierda, realce.intensidad(0)),
            Space::new().width(Length::Fixed(HUECO_FILA)),
            boton(derecha, realce.intensidad(1)),
        ],
    ]
    .into()
}

/// Como [`fila_en`] pero con la lista desplazada: fuera de la ventana no hay
/// fila, aunque debajo de ella siga habiendo más.
// añadido en la 0.2.0
pub fn fila_en_ventana(x: f32, y: f32, y0: f32, n: usize, desplazamiento: f32) -> Option<usize> {
    if y >= y0 + alto_ventana(n) {
        return None;
    }
    fila_en(x, y + desplazamiento, y0, n)
}

/// Sobre qué botón del pie cae un punto, si el pie empieza en `y0`: `false` el
/// izquierdo y `true` el derecho.
// 0.1.0
pub fn pie_en(x: f32, y: f32, y0: f32) -> Option<bool> {
    if y < y0 || y > y0 + PIE_BOTON {
        return None;
    }
    (x > MARGEN + GRUPO && x < ANCHO - MARGEN - GRUPO).then(|| x > ANCHO / 2.0)
}

/// Dónde cae un punto dentro de una lista que empieza en `y0`.
///
/// El hueco entre filas no es de nadie: pulsarlo no debe activar la de al lado.
// 0.1.0
pub fn fila_en(x: f32, y: f32, y0: f32, n: usize) -> Option<usize> {
    if x < MARGEN + GRUPO || x > ANCHO - MARGEN - GRUPO || y < y0 {
        return None;
    }
    let rel = y - y0;
    let i = (rel / (FILA + HUECO_FILA)) as usize;
    if rel - i as f32 * (FILA + HUECO_FILA) > FILA {
        return None;
    }
    (i < n).then_some(i)
}

#[cfg(test)]
mod tests {
    /// Bajar con el teclado desplaza la lista solo cuando la fila se saldría,
    /// y subir al principio la deja a cero.
    #[test]
    fn el_teclado_recorre_y_desplaza_lo_justo() {
        let mut señalada = tema::Realce::nuevo();
        let mut desp = 0.0;
        for _ in 0..4 {
            mover_seleccion(&mut señalada, &mut desp, 7, 1);
        }
        assert_eq!(señalada.actual(), Some(3));
        assert_eq!(desp, 0.0, "la cuarta fila aún cabe");
        mover_seleccion(&mut señalada, &mut desp, 7, 1);
        assert_eq!(desp, FILA + HUECO_FILA, "la quinta empuja una fila");
        for _ in 0..20 {
            mover_seleccion(&mut señalada, &mut desp, 7, 1);
        }
        assert_eq!(señalada.actual(), Some(6), "no da la vuelta");
        assert_eq!(desp, maximo_desplazamiento(7));
        for _ in 0..20 {
            mover_seleccion(&mut señalada, &mut desp, 7, -1);
        }
        assert_eq!((señalada.actual(), desp), (Some(0), 0.0));
    }

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

    /// Con más filas que ventana la lista se desplaza hasta el final y ni un
    /// píxel más; con menos, no hay nada que mover.
    #[test]
    fn la_lista_se_desplaza_dentro_de_sus_limites() {
        let mut y = 0.0;
        assert!(!desplazar(&mut y, 50.0, VISIBLES), "cabe entera");
        let n = VISIBLES + 3;
        assert!(desplazar(&mut y, 1000.0, n));
        assert_eq!(y, 3.0 * (FILA + HUECO_FILA), "tres filas de más");
        assert!(!desplazar(&mut y, 10.0, n), "ya está abajo");
        assert!(desplazar(&mut y, -1000.0, n));
        assert_eq!(y, 0.0);
    }

    /// Una fila desplazada fuera de la ventana no se puede pulsar aunque
    /// siga existiendo debajo.
    #[test]
    fn solo_se_pulsa_lo_que_se_ve() {
        let n = VISIBLES + 2;
        let y0 = 100.0;
        let ventana = alto_ventana(n);
        assert_eq!(fila_en_ventana(50.0, y0 + 1.0, y0, n, 0.0), Some(0));
        assert_eq!(fila_en_ventana(50.0, y0 + ventana + 1.0, y0, n, 0.0), None);
        let un_paso = FILA + HUECO_FILA;
        assert_eq!(fila_en_ventana(50.0, y0 + 1.0, y0, n, un_paso), Some(1));
    }

    #[test]
    fn el_hueco_entre_filas_no_es_de_nadie() {
        assert_eq!(fila_en(50.0, 100.0, 100.0, 2), Some(0));
        assert_eq!(fila_en(50.0, 100.0 + FILA + 2.0, 100.0, 2), None);
        assert_eq!(
            fila_en(50.0, 100.0 + FILA + HUECO_FILA + 1.0, 100.0, 2),
            Some(1)
        );
        assert_eq!(fila_en(50.0, 99.0, 100.0, 2), None, "por encima");
        assert_eq!(
            fila_en(MARGEN, 110.0, 100.0, 2),
            None,
            "fuera por la izquierda"
        );
    }
}
