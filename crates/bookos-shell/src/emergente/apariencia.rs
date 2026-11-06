//! Apariencia: el tema claro u oscuro y el color de acento.
//!
//! Es la primera tarjeta del shell que **escribe** en la configuración algo que
//! no es una lista de aplicaciones: al elegir aquí, el compositor aplica el
//! cambio en caliente y lo guarda en `panel.conf`. El escritorio no se
//! reinicia y no hay que editar el fichero a mano, que era la única forma de
//! cambiar de tema hasta ahora.
//!
//! **Por qué vive en el shell y no en `bookos-settings`.** El acento es un
//! global del proceso del compositor (`tema::aplicar_acento`): quien lo cambie
//! desde fuera tendría que avisar por algún camino que hoy no existe —el shell
//! no tiene D-Bus ni vigila el fichero—, y el panel se quedaría con la paleta
//! anterior hasta reiniciar la sesión. Cuando `bookos-settings` tenga por dónde
//! decírselo, esta tarjeta sigue valiendo: es el mismo control, más a mano.

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema::{self, Acento, ModoTema, Realce};
use crate::view::PanelElement;

use super::{Ancla, Tecla};

/// El ancho de las demás tarjetas del shell.
const ANCHO: f32 = 335.0;
const MARGEN: f32 = 21.0;
const CABECERA: f32 = 40.0;
/// Alto de la muestra de un tema, con su rótulo debajo.
const MUESTRA: f32 = 78.0;
const MUESTRA_HUECO: f32 = 11.0;
/// Alto del renglón «Color de acento».
const ROTULO: f32 = 30.0;
/// Lado de la casilla de un color: el círculo de 30 más su anillo.
const CASILLA: f32 = 42.0;
const COLUMNAS: usize = 6;
const CIRCULO: f32 = 30.0;
/// El divisor y el aire que lo rodea.
const SEPARACION: f32 = 17.0;
/// Las miniaturas de fondo van en rejilla de tres columnas, del mismo ancho que
/// las muestras de tema de arriba, y no repartiendo la fila entre las familias
/// que haya: con cinco salían miniaturas de 51 px casi cuadradas, con más aún
/// menores, y no casaban con nada de lo que tienen encima. Con más familias la
/// rejilla gana filas en vez de encoger.
const FONDO_COLUMNAS: usize = 3;
/// Hueco entre dos miniaturas de fondo, el mismo que entre las muestras.
const FONDO_HUECO: f32 = MUESTRA_HUECO;
/// Lo que se separa el anillo de elegida de la imagen. Sin este aire el anillo
/// cae encima de la foto y con el acento azul sobre la miniatura azul no se ve.
const HUECO_ANILLO: f32 = 4.0;

/// Los tres modos, en el orden en que se enseñan.
const MODOS: [ModoTema; 3] = [ModoTema::Claro, ModoTema::Oscuro, ModoTema::Automatico];

/// Ancho de una muestra de tema. Tres, con un hueco entre cada dos.
// 0.1.0
fn ancho_muestra() -> f32 {
    (ANCHO - MARGEN * 2.0 - MUESTRA_HUECO * (MODOS.len() - 1) as f32) / MODOS.len() as f32
}

/// Paso horizontal de la rejilla de colores: lo que sobra se reparte entre las
/// cuatro calles, no se acumula a la derecha.
// 0.1.0
fn paso() -> f32 {
    let libre = ANCHO - MARGEN * 2.0 - CASILLA * COLUMNAS as f32;
    CASILLA + libre / (COLUMNAS - 1) as f32
}

// 0.1.0
fn filas() -> usize {
    Acento::TODOS.len().div_ceil(COLUMNAS)
}

/// Ancho de una miniatura de fondo: el de una muestra de tema.
// 0.1.0
fn ancho_fondo() -> f32 {
    ancho_muestra()
}

/// Alto de una miniatura: la proporción 16:10 de los fondos que trae BookOS.
// 0.1.0
fn alto_fondo() -> f32 {
    (ancho_fondo() * 10.0 / 16.0).round()
}

/// Lo que ocupa la rejilla de `n` familias, con el hueco entre filas.
// 0.1.0
fn alto_fondos(n: usize) -> f32 {
    let filas = n.div_ceil(FONDO_COLUMNAS) as f32;
    filas * alto_fondo() + (filas - 1.0).max(0.0) * FONDO_HUECO
}

pub struct Apariencia {
    /// Lo elegido. No se lee de `tema::actual()` en cada `view` porque el
    /// cambio lo aplica el compositor y puede tardar un frame en volver: la
    /// tarjeta se marca a sí misma en cuanto se pulsa.
    ///
    /// Es el **modo**, no el tema pintado: con «Automático» puesto, `actual()`
    /// dice claro u oscuro según la hora y marcar esa muestra convertiría el
    /// automático en fijo con solo abrir la tarjeta.
    modo: ModoTema,
    acento: Acento,
    /// Lo señalado por el puntero: las dos muestras de tema y la rejilla.
    hover_tema: Realce,
    hover_color: Realce,
    /// Lo elegido, animado: el anillo crece en el nuevo y se apaga en el
    /// anterior en vez de saltar de un sitio a otro.
    marca_tema: Realce,
    marca_color: Realce,
    /// Las parejas de fondos instaladas, leídas **al abrir** y no en cada
    /// `view`: recorrer tres directorios sesenta veces por segundo para una
    /// lista que no cambia mientras la tarjeta está abierta es trabajo por nada.
    fondos: Vec<crate::fondos::Familia>,
    /// La miniatura de cada familia ya en píxeles y con las esquinas
    /// recortadas, en el mismo orden que `fondos`. Ver [`vistas_de`].
    vistas: Vec<Option<iced_core::image::Handle>>,
    hover_fondo: Realce,
    marca_fondo: Realce,
    /// La casilla del acento automático ya rasterizada, con la paleta de la
    /// que salió. Es interior porque la paleta cambia con la tarjeta abierta
    /// —al elegir fondo o tema, y lo aplica el compositor un frame después— y
    /// quien se entera es `view`, que solo tiene `&self`.
    franjas: std::cell::RefCell<Option<([Color; 3], iced_core::image::Handle)>>,
}

impl Apariencia {
    /// `modo` es lo que dice la configuración, que la tarjeta no puede deducir:
    /// `tema::actual()` solo sabe qué se está pintando.
    // 0.1.0
    // modified 0.1.1 -- franjas para la muestra del acento automático
    pub fn new(modo: ModoTema) -> Self {
        let acento = tema::acento_actual();
        let mut marca_tema = Realce::nuevo();
        let mut marca_color = Realce::nuevo();
        // Al abrirse ya está elegido: sin esto la tarjeta aparece con todo
        // apagado y las marcas entrando, como si nadie hubiera elegido nada.
        marca_tema.señalar(MODOS.iter().position(|m| *m == modo));
        marca_color.señalar(indice(acento));
        let (fondos, elegido) = Self::fondos_para(tema::es_claro(), acento);
        let mut marca_fondo = Realce::nuevo();
        marca_fondo.señalar(elegido);
        marca_tema.terminar();
        marca_color.terminar();
        marca_fondo.terminar();
        Self {
            modo,
            acento,
            hover_tema: Realce::nuevo(),
            hover_color: Realce::nuevo(),
            marca_tema,
            marca_color,
            vistas: vistas_de(&fondos),
            fondos,
            hover_fondo: Realce::nuevo(),
            marca_fondo,
            franjas: Default::default(),
        }
    }

    /// La imagen de la casilla automática, rehecha solo si cambió la paleta:
    /// un `Handle` nuevo en cada `view` obligaría a iced a subirla otra vez
    /// en cada fotograma.
    // 0.1.1
    fn franjas(&self) -> iced_core::image::Handle {
        let paleta = tema::paleta_automatica();
        let mut guardadas = self.franjas.borrow_mut();
        match guardadas.as_ref() {
            Some((de, imagen)) if *de == paleta => imagen.clone(),
            _ => {
                let imagen = imagen_de_franjas(paleta);
                *guardadas = Some((paleta, imagen.clone()));
                imagen
            }
        }
    }

    /// Los fondos instalados para ese tema, ordenados por afinidad con el
    /// acento, y el índice del que ya está puesto.
    ///
    /// Se compara por **nombre de familia** y no por ruta: con el tema oscuro
    /// puesto, el fichero cargado es el `_dark` de la pareja, y comparar rutas
    /// no encontraría la miniatura de su propia familia.
    // 0.1.0
    fn fondos_para(es_claro: bool, acento: Acento) -> (Vec<crate::fondos::Familia>, Option<usize>) {
        let mut fondos = crate::fondos::instaladas(es_claro);
        ordenar_por_afinidad(&mut fondos, acento);
        let puesta = crate::fondos::elegida();
        let elegido = puesta
            .as_deref()
            .and_then(|nombre| fondos.iter().position(|f| f.nombre == nombre));
        (fondos, elegido)
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        let mut alto =
            CABECERA + MUESTRA + SEPARACION + ROTULO + CASILLA * filas() as f32 + MARGEN * 2.0;
        // Sin fondos instalados la sección no se dibuja: un rótulo con una fila
        // vacía debajo no es «no hay ninguno», es «esto está roto».
        if !self.fondos.is_empty() {
            alto += SEPARACION + ROTULO + alto_fondos(self.fondos.len());
        }
        (ANCHO, alto)
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        // Cuelga del panel por la izquierda, como el menú del que se abre.
        Ancla::BajoElPanel { x: 8.0 }
    }

    /// ¿Se está moviendo algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.hover_tema.animando()
            || self.hover_color.animando()
            || self.marca_tema.animando()
            || self.marca_color.animando()
            || self.hover_fondo.animando()
            || self.marca_fondo.animando()
    }

    /// `y` donde empieza la fila de muestras de tema.
    // 0.1.0
    fn y_muestras(&self) -> f32 {
        MARGEN + CABECERA
    }

    // 0.1.0
    fn y_rejilla(&self) -> f32 {
        self.y_muestras() + MUESTRA + SEPARACION + ROTULO
    }

    // 0.1.0
    fn muestra_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_muestras();
        if y < y0 || y > y0 + MUESTRA || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        let rel = x - MARGEN;
        let paso = ancho_muestra() + MUESTRA_HUECO;
        let i = (rel / paso) as usize;
        // El hueco entre dos muestras no es de nadie: pulsar en la calle no
        // puede elegir la de la izquierda.
        if rel - i as f32 * paso > ancho_muestra() {
            return None;
        }
        (i < MODOS.len()).then_some(i)
    }

    /// `y` donde empieza la fila de miniaturas de fondo.
    // 0.1.0
    fn y_fondos(&self) -> f32 {
        self.y_rejilla() + CASILLA * filas() as f32 + SEPARACION + ROTULO
    }

    // 0.1.0
    fn fondo_en(&self, x: f32, y: f32) -> Option<usize> {
        if self.fondos.is_empty() {
            return None;
        }
        let y0 = self.y_fondos();
        if y < y0 || y > y0 + alto_fondos(self.fondos.len()) || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        let (paso_x, paso_y) = (ancho_fondo() + FONDO_HUECO, alto_fondo() + FONDO_HUECO);
        let (rel_x, rel_y) = (x - MARGEN, y - y0);
        let (col, fila) = ((rel_x / paso_x) as usize, (rel_y / paso_y) as usize);
        // Las calles entre miniaturas no son de nadie.
        if rel_x - col as f32 * paso_x > ancho_fondo()
            || rel_y - fila as f32 * paso_y > alto_fondo()
        {
            return None;
        }
        let i = fila * FONDO_COLUMNAS + col;
        (col < FONDO_COLUMNAS && i < self.fondos.len()).then_some(i)
    }

    // 0.1.0
    fn color_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_rejilla();
        if y < y0 || x < MARGEN {
            return None;
        }
        let (col, fila) = (
            ((x - MARGEN) / paso()) as usize,
            ((y - y0) / CASILLA) as usize,
        );
        // Fuera de la casilla, en la calle que la separa de la siguiente: no
        // cuenta, o pulsar entre dos colores elegiría el de la izquierda.
        if x - MARGEN - col as f32 * paso() > CASILLA || col >= COLUMNAS {
            return None;
        }
        let i = fila * COLUMNAS + col;
        (i < Acento::TODOS.len()).then_some(i)
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let (m, c, f) = match punto {
            Some((x, y)) => (
                self.muestra_en(x, y),
                self.color_en(x, y),
                self.fondo_en(x, y),
            ),
            None => (None, None, None),
        };
        // Los tres `señalar` **siempre**, sin cortocircuito: con `||` los de
        // después no se evalúan cuando el primero cambia y la rejilla se queda
        // con un color encendido al salir el ratón por arriba.
        let a = self.hover_tema.señalar(m);
        let b = self.hover_color.señalar(c);
        let d = self.hover_fondo.señalar(f);
        a || b || d
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.muestra_en(x, y) {
            self.modo = MODOS[i];
            self.marca_tema.señalar(Some(i));
            // Con «Automático» no se sabe aquí qué toca sin la hora y la
            // configuración de horas, que esta tarjeta no lleva: se deja tal
            // cual hasta que el compositor la reabra. Con Claro u Oscuro sí
            // se sabe sin ambigüedad, y las miniaturas de fondo son la
            // pareja del tema anterior si no se refrescan aquí.
            if let ModoTema::Claro | ModoTema::Oscuro = self.modo {
                let es_claro = self.modo == ModoTema::Claro;
                let (fondos, elegido) = Self::fondos_para(es_claro, self.acento);
                self.vistas = vistas_de(&fondos);
                self.fondos = fondos;
                self.marca_fondo.señalar(elegido);
                self.marca_fondo.terminar();
            }
            return Some(self.accion());
        }
        if let Some(i) = self.fondo_en(x, y) {
            self.marca_fondo.señalar(Some(i));
            let familia = &self.fondos[i];
            // El fondo va en su propia acción y no en `Apariencia`: son dos
            // claves distintas del fichero y elegir un fondo no toca el tema.
            return Some(Accion::Fondo {
                claro: familia.claro.clone(),
                oscuro: familia.oscuro.clone(),
            });
        }
        let i = self.color_en(x, y)?;
        self.acento = Acento::TODOS[i];
        self.marca_color.señalar(Some(i));
        Some(self.accion())
    }

    // 0.1.0
    fn accion(&self) -> Accion {
        Accion::Apariencia {
            modo: self.modo,
            acento: self.acento,
        }
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- muestra el acento automático con franjas
    pub fn view(&self) -> PanelElement<'_> {
        let mut rejilla = column![];
        for f in 0..filas() {
            let mut fila = row![];
            for c in 0..COLUMNAS {
                let i = f * COLUMNAS + c;
                let Some(acento) = Acento::TODOS.get(i).copied() else {
                    break;
                };
                if c > 0 {
                    fila = fila.push(Space::new().width(Length::Fixed(paso() - CASILLA)));
                }
                fila = fila.push(casilla(
                    acento,
                    (acento == Acento::Automatico).then(|| self.franjas()),
                    self.marca_color.intensidad(i),
                    self.hover_color.intensidad(i),
                ));
            }
            rejilla = rejilla.push(fila);
        }

        let contenido = column![
            container(text("Apariencia").size(tema::T_TITULO).color(tema::texto()))
                .height(Length::Fixed(CABECERA))
                .center_y(Length::Fixed(CABECERA)),
            MODOS.iter().enumerate().fold(row![], |fila, (i, modo)| {
                let fila = if i > 0 {
                    fila.push(Space::new().width(Length::Fixed(MUESTRA_HUECO)))
                } else {
                    fila
                };
                fila.push(muestra(
                    *modo,
                    self.marca_tema.intensidad(i),
                    self.hover_tema.intensidad(i),
                ))
            }),
            separador(),
            container(
                text("Color de acento")
                    .size(tema::T_CUERPO)
                    .color(tema::TEXTO2)
            )
            .height(Length::Fixed(ROTULO))
            .center_y(Length::Fixed(ROTULO)),
            rejilla,
        ];

        // La sección de fondos solo si hay alguno: un rótulo con una fila vacía
        // debajo no dice «no hay ninguno», dice «esto está roto».
        let contenido = if self.fondos.is_empty() {
            contenido
        } else {
            let rejilla = self.vistas.chunks(FONDO_COLUMNAS).enumerate().fold(
                column![],
                |rejilla, (f, trozo)| {
                    let rejilla = if f > 0 {
                        rejilla.push(Space::new().height(Length::Fixed(FONDO_HUECO)))
                    } else {
                        rejilla
                    };
                    rejilla.push(trozo.iter().enumerate().fold(row![], |fila, (c, vista)| {
                        let i = f * FONDO_COLUMNAS + c;
                        let fila = if c > 0 {
                            fila.push(Space::new().width(Length::Fixed(FONDO_HUECO)))
                        } else {
                            fila
                        };
                        fila.push(miniatura(
                            vista.as_ref(),
                            ancho_fondo(),
                            self.marca_fondo.intensidad(i),
                            self.hover_fondo.intensidad(i),
                        ))
                    }))
                },
            );
            contenido
                .push(separador())
                .push(
                    container(text("Fondo").size(tema::T_CUERPO).color(tema::TEXTO2))
                        .height(Length::Fixed(ROTULO))
                        .center_y(Length::Fixed(ROTULO)),
                )
                .push(rejilla)
        };

        super::control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Pone delante el fondo que acompaña al acento elegido.
///
/// `fondos::instaladas` los devuelve por nombre —blue, ember, pine, purple— y
/// alfabético no le dice nada al ojo: con el acento en verde, el fondo que le
/// va queda el tercero de cuatro. Ordenando por cercanía de tono, la primera
/// miniatura es siempre la pareja natural.
///
/// **Ordena, no elige.** El fondo se sigue cambiando a mano y elegir un acento
/// no lo toca: son dos acciones distintas a propósito, y mover el fondo por
/// detrás pisaría el que el usuario hubiera puesto. Esto solo pone el candidato
/// donde se mira primero.
///
/// Grafito no tiene tono —es gris— y con él se deja el orden alfabético: no hay
/// ningún fondo que le pegue más que otro. Lo mismo con las familias sin SVG,
/// que van al final porque de ellas no se sabe el color sin decodificar el PNG.
// 0.1.0
fn ordenar_por_afinidad(fondos: &mut [crate::fondos::Familia], acento: Acento) {
    let Some(objetivo) = tono_de_acento(acento) else {
        return;
    };
    fondos.sort_by(|a, b| {
        let d = |f: &crate::fondos::Familia| {
            crate::fondos::tono(f)
                .map(|t| crate::fondos::distancia_de_tono(t, objetivo))
                // Sin vectorial no se sabe el tono: detrás de cualquiera que sí.
                .unwrap_or(f32::MAX)
        };
        d(a).total_cmp(&d(b)).then_with(|| a.nombre.cmp(&b.nombre))
    });
}

/// El tono del acento, o `None` si es gris y no tiene.
///
/// El umbral es de croma y no de cero exacto: el Grafito oscuro es `#a1a1a6`,
/// con cinco niveles de diferencia entre canales, y la cuenta le sacaría un
/// 240° azul perfectamente formal que dejaría el fondo azul el primero por un
/// gris. Por debajo de 0,08 no hay tono que signifique nada.
// 0.1.0
fn tono_de_acento(acento: Acento) -> Option<f32> {
    let c = acento.color();
    let (max, min) = (c.r.max(c.g).max(c.b), c.r.min(c.g).min(c.b));
    let croma = max - min;
    if croma < 0.08 {
        return None;
    }
    let h = if max == c.r {
        60.0 * (((c.g - c.b) / croma) % 6.0)
    } else if max == c.g {
        60.0 * ((c.b - c.r) / croma + 2.0)
    } else {
        60.0 * ((c.r - c.g) / croma + 4.0)
    };
    Some((h + 360.0) % 360.0)
}

// 0.1.0
fn indice(acento: Acento) -> Option<usize> {
    Acento::TODOS.iter().position(|a| *a == acento)
}

/// Los dos colores de un tema, escritos aquí a mano.
///
/// Son los mismos `bg` y `card` de la tabla de tokens, pero no se le pueden
/// preguntar a `tema::bg()`: la muestra enseña el tema que **no** está puesto.
// 0.1.0
fn colores(claro: bool) -> (Color, Color) {
    if claro {
        (Color::from_rgb(0.949, 0.949, 0.969), Color::WHITE)
    } else {
        (Color::BLACK, Color::from_rgb(0.110, 0.110, 0.118))
    }
}

/// Un trozo de escritorio en miniatura: la barra del panel arriba y una tarjeta
/// debajo, que es lo que distingue de un vistazo un tema del otro.
// 0.1.0
fn mini<'a>(
    claro: bool,
    ancho: f32,
    alto: f32,
    radio: iced_core::border::Radius,
) -> PanelElement<'a> {
    let (fondo, tarjeta) = colores(claro);
    // La barra tiene su propio gris y no el de la tarjeta: era `tarjeta`, que
    // en oscuro es el `#1c1c1e` de la emergente donde va la muestra, y en
    // claro el blanco de la tarjeta clara. Se fundía con el fondo, y como ella
    // es la que lleva las esquinas de arriba, la muestra se veía cortada en
    // recto por arriba.
    let barra = if claro {
        Color::from_rgb(0.820, 0.820, 0.839)
    } else {
        Color::from_rgb(0.227, 0.227, 0.235)
    };
    // La barra del panel no se dibuja como una pieza de 8 px con las esquinas
    // de arriba redondeadas: tiny-skia limita el radio a la mitad del lado más
    // corto, así que un radio de 6 en 8 px de alto se quedaría en 4 y la
    // barra asomaba en pico por encima de la curva. Se hace al revés: el
    // contenedor de fuera pinta el color de la barra con el radio entero, y el
    // cuerpo de debajo tapa el resto redondeando solo por abajo.
    let abajo = iced_core::border::Radius {
        top_left: 0.0,
        top_right: 0.0,
        ..radio
    };
    let cuerpo = container(
        container(Space::new())
            .width(Length::Fixed(ancho * 0.5))
            .height(Length::Fixed(14.0))
            .style(move |_| container::Style {
                background: Some(tarjeta.into()),
                border: Border {
                    radius: tema::R_CHIP.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .padding(8)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::Style {
        background: Some(fondo.into()),
        border: Border {
            radius: abajo,
            ..Default::default()
        },
        ..Default::default()
    });
    container(column![Space::new().height(Length::Fixed(8.0)), cuerpo])
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto))
        .style(move |_| container::Style {
            background: Some(barra.into()),
            border: Border {
                radius: radio,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// La muestra de un modo.
///
/// «Automático» se dibuja partido: la mitad clara a la izquierda y la oscura a
/// la derecha. Es lo mismo que enseña macOS, y dice lo que hace sin necesitar
/// el rótulo. El corte es vertical y no en diagonal porque una diagonal pide un
/// lienzo con su propio dibujado y aquí basta con dos contenedores.
// 0.1.0
fn muestra<'a>(cual: ModoTema, elegida: f32, señalada: f32) -> PanelElement<'a> {
    let ancho = ancho_muestra();
    let alto_lienzo = MUESTRA - 22.0;

    use iced_core::border::Radius;
    // El radio no puede pasar del alto de la barra (8): el cuerpo va recto por
    // arriba, y con R_BOTON (12) sus esquinas asomaban por fuera de la curva,
    // que seguía bajando 4 px por debajo de la barra.
    let entero = Radius::from(tema::R_CHIP);
    let izquierda = Radius::default()
        .top_left(tema::R_CHIP)
        .bottom_left(tema::R_CHIP);
    let derecha = Radius::default()
        .top_right(tema::R_CHIP)
        .bottom_right(tema::R_CHIP);

    let dentro: PanelElement<'a> = match cual {
        ModoTema::Claro => mini(true, ancho, alto_lienzo, entero),
        ModoTema::Oscuro => mini(false, ancho, alto_lienzo, entero),
        // Cada mitad lleva su propio fondo, así que el lienzo de fuera no pinta
        // ninguno: si lo hiciera, asomaría por la junta al redondear.
        ModoTema::Automatico => row![
            mini(true, ancho / 2.0, alto_lienzo, izquierda),
            mini(false, ancho / 2.0, alto_lienzo, derecha),
        ]
        .into(),
    };

    let lienzo = container(dentro)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto_lienzo))
        .clip(true)
        .style(move |_| container::Style {
            border: Border {
                radius: tema::R_CHIP.into(),
                // El marco de elegido crece con la animación en vez de
                // aparecer: 2 px que salen de golpe se leen como un parpadeo.
                width: 2.0 * elegida,
                color: tema::alfa(tema::acento(), elegida),
            },
            ..Default::default()
        });

    let rotulo = text(match cual {
        ModoTema::Claro => "Claro",
        ModoTema::Oscuro => "Oscuro",
        ModoTema::Automatico => "Automático",
    })
    .size(tema::T_PEQUENO)
    // El rótulo es de la tarjeta, no de la muestra: va con la tinta del tema
    // que está puesto. Señalado sube de secundario a normal; elegido, al acento.
    .color(tema::mezclar(
        tema::mezclar(tema::TEXTO2, tema::texto(), señalada),
        tema::acento(),
        elegida,
    ));

    container(
        column![lienzo, Space::new().height(Length::Fixed(4.0)), rotulo]
            .align_x(iced_core::alignment::Horizontal::Center),
    )
    .width(Length::Fixed(ancho))
    .height(Length::Fixed(MUESTRA))
    .into()
}

/// Las miniaturas de las familias, rasterizadas **una vez** al abrir la
/// tarjeta y con las esquinas ya recortadas.
///
/// Se dibuja el **SVG** y no la foto: decodificar los cuatro PNG de 2880×1800
/// cuesta 152 ms medidos, y los SVG de al lado son de cuatro kilobytes. Pero no
/// se le pasan a iced como SVG: su renderizador de CPU no recorta las imágenes
/// por el radio, y la miniatura salía con las esquinas en pico dentro del
/// anillo redondeado. Tapar las esquinas con una capa encima tampoco sirvió:
/// iced coloca un SVG y una imagen con un píxel de diferencia —medido: el SVG
/// empezaba en (100, 320) y la capa en (101, 321)— y quedaba una raya.
// 0.1.0
fn vistas_de(fondos: &[crate::fondos::Familia]) -> Vec<Option<iced_core::image::Handle>> {
    let (iw, ih) = (
        ancho_fondo() - HUECO_ANILLO * 2.0,
        alto_fondo() - HUECO_ANILLO * 2.0,
    );
    fondos
        .iter()
        .map(|f| {
            f.vista
                .as_deref()
                .and_then(|ruta| vista_redondeada(ruta, iw, ih))
        })
        .collect()
}

// 0.1.0
fn vista_redondeada(
    ruta: &std::path::Path,
    ancho: f32,
    alto: f32,
) -> Option<iced_core::image::Handle> {
    use resvg::tiny_skia::{Color as Tinta, Pixmap, Transform};
    // A 3× del tamaño lógico: cubre hasta escala 3 sin que la miniatura se
    // vea borrosa, y son cuatro imágenes de 200×100.
    const RESOLUCION: f32 = 3.0;
    let (pw, ph) = (
        (ancho * RESOLUCION).round() as u32,
        (alto * RESOLUCION).round() as u32,
    );
    let datos = std::fs::read(ruta).ok()?;
    let arbol = resvg::usvg::Tree::from_data(&datos, &resvg::usvg::Options::default()).ok()?;
    let t = arbol.size();
    // Como `ContentFit::Cover`: llena el hueco y recorta lo que sobra.
    let escala = (pw as f32 / t.width()).max(ph as f32 / t.height());
    let dx = (pw as f32 - t.width() * escala) / 2.0;
    let dy = (ph as f32 - t.height() * escala) / 2.0;
    let mut pixmap = Pixmap::new(pw, ph)?;
    // Opaco, como el fondo de verdad: así el alfa premultiplicado de tiny-skia
    // y el recto que espera iced coinciden en todo menos en las esquinas.
    pixmap.fill(Tinta::BLACK);
    resvg::render(
        &arbol,
        Transform::from_row(escala, 0.0, 0.0, escala, dx, dy),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.take();
    crate::redondear_esquinas(
        &mut rgba,
        pw,
        ph,
        tema::R_BOTON * RESOLUCION,
        crate::Alfa::Recto,
    );
    Some(iced_core::image::Handle::from_rgba(pw, ph, rgba))
}

/// La miniatura de una familia de fondos. Una familia sin vectorial se queda
/// con un rectángulo liso: se puede elegir igual, que es lo que importa.
// 0.1.0
fn miniatura<'a>(
    vista: Option<&iced_core::image::Handle>,
    ancho: f32,
    elegida: f32,
    señalada: f32,
) -> PanelElement<'a> {
    // La imagen va **dentro** de un margen y el anillo por fuera, igual que los
    // círculos de acento. Pegado a la imagen no se vería: el anillo es del color
    // de acento y con el acento azul sobre la miniatura azul son el mismo color.
    let (iw, ih) = (
        ancho - HUECO_ANILLO * 2.0,
        alto_fondo() - HUECO_ANILLO * 2.0,
    );
    let imagen: PanelElement<'a> = match vista {
        Some(vista) => iced_widget::image(vista.clone())
            .width(Length::Fixed(iw))
            .height(Length::Fixed(ih))
            .content_fit(iced_core::ContentFit::Fill)
            .into(),
        None => container(Space::new())
            .width(Length::Fixed(iw))
            .height(Length::Fixed(ih))
            .style(move |_| container::Style {
                background: Some(tema::hover().into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    };

    container(imagen)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto_fondo()))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(alto_fondo()))
        .style(move |_| container::Style {
            border: Border {
                radius: (tema::R_BOTON + HUECO_ANILLO).into(),
                // El anillo de elegida crece con la animación, igual que en las
                // muestras de tema; el de señalada es más flojo y no es acento,
                // que se reserva para lo que está puesto.
                width: 2.0 * elegida.max(señalada),
                color: tema::mezclar(
                    tema::alfa(tema::texto(), 0.35 * señalada),
                    tema::acento(),
                    elegida,
                ),
            },
            ..Default::default()
        })
        .into()
}

/// El círculo del acento automático: los colores del fondo en tres franjas, el
/// dominante —el que se aplica— arriba.
///
/// Es una imagen y no tres contenedores porque tiny-skia limita el radio a la
/// mitad del lado más corto: una franja de 10 px no puede llevar la curva de un
/// círculo de 30.
// 0.1.1
fn imagen_de_franjas(paleta: [Color; 3]) -> iced_core::image::Handle {
    // A 3× del círculo, como las miniaturas de fondo.
    const LADO: u32 = CIRCULO as u32 * 3;
    let mut rgba: Vec<u8> = (0..LADO)
        .flat_map(|y| {
            let franja = paleta[(y * paleta.len() as u32 / LADO) as usize];
            std::iter::repeat_n(franja.into_rgba8(), LADO as usize)
        })
        .flatten()
        .collect();
    crate::redondear_esquinas(&mut rgba, LADO, LADO, LADO as f32 / 2.0, crate::Alfa::Recto);
    iced_core::image::Handle::from_rgba(LADO, LADO, rgba)
}

/// Una casilla de la rejilla: el círculo del color, con su anillo si está
/// elegido y su halo si está señalado. `franjas` es la imagen del automático.
// 0.1.0
// modified 0.1.1 -- la casilla puede llevar las franjas del acento automático
fn casilla<'a>(
    acento: Acento,
    franjas: Option<iced_core::image::Handle>,
    elegido: f32,
    señalado: f32,
) -> PanelElement<'a> {
    // El color de la casilla es el del acento **en el tema que está puesto**,
    // que es como se va a ver si se elige.
    let color = acento.color();
    // Señalado, el círculo crece 3 px sin mover a los vecinos: la casilla mide
    // 42 y el círculo 30, así que hay sitio de sobra dentro.
    let lado = CIRCULO + 3.0 * señalado;
    let punto: PanelElement<'a> = match franjas {
        Some(franjas) => iced_widget::image(franjas)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .into(),
        None => container(Space::new())
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    };
    container(punto)
        .width(Length::Fixed(CASILLA))
        .height(Length::Fixed(CASILLA))
        .center_x(Length::Fixed(CASILLA))
        .center_y(Length::Fixed(CASILLA))
        .style(move |_| container::Style {
            border: Border {
                radius: tema::R_PILL.into(),
                // El anillo va del color del propio acento y no del acento
                // puesto: el que se está eligiendo todavía no lo es.
                width: 2.0 * elegido,
                color: tema::alfa(color, elegido),
            },
            ..Default::default()
        })
        .into()
}

// 0.1.0
fn separador<'a>() -> PanelElement<'a> {
    container(
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            }),
    )
    .height(Length::Fixed(SEPARACION))
    .center_y(Length::Fixed(SEPARACION))
    .align_y(Vertical::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Las diez casillas tienen que caber en la rejilla y responder cada una
    /// en su sitio: pulsar entre dos colores no elige ninguno.
    #[test]
    fn la_rejilla_reparte_los_diez_colores() {
        let a = Apariencia::new(ModoTema::Oscuro);
        let y0 = a.y_rejilla();
        assert_eq!(a.color_en(MARGEN + 1.0, y0 + 1.0), Some(0));
        assert_eq!(a.color_en(MARGEN + paso() + 1.0, y0 + 1.0), Some(1));
        assert_eq!(
            a.color_en(MARGEN + 1.0, y0 + CASILLA + 1.0),
            Some(COLUMNAS),
            "la segunda fila empieza en el sexto color"
        );
        assert_eq!(
            a.color_en(MARGEN + CASILLA + 3.0, y0 + 1.0),
            None,
            "la calle entre casillas no elige nada"
        );
        assert_eq!(a.color_en(MARGEN - 5.0, y0 + 1.0), None);
        // Y la última casilla existe: con COLUMNAS mal puesto, el décimo color
        // quedaría fuera de la tarjeta y sería imposible de elegir.
        let ultimo = Acento::TODOS.len() - 1;
        let (f, c) = (ultimo / COLUMNAS, ultimo % COLUMNAS);
        assert_eq!(
            a.color_en(
                MARGEN + c as f32 * paso() + 1.0,
                y0 + f as f32 * CASILLA + 1.0
            ),
            Some(ultimo)
        );
        assert!(a.y_rejilla() + CASILLA * filas() as f32 <= a.size().1);
    }

    #[test]
    fn elegir_un_color_lo_pide_y_lo_marca() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        let y0 = a.y_rejilla();
        let accion = a.pulsar(MARGEN + paso() * 2.0 + 1.0, y0 + 1.0);
        assert_eq!(
            accion,
            Some(Accion::Apariencia {
                modo: ModoTema::Oscuro,
                acento: Acento::TODOS[2],
            })
        );
        assert_eq!(a.marca_color.actual(), Some(2));
    }

    /// El fondo que acompaña al acento sale el primero, y el gris no reordena.
    ///
    /// Se comprueba con los tonos, no con los nombres: si mañana entra un
    /// fondo nuevo, el test sigue diciendo lo que importa —que el primero es
    /// el más cercano— en vez de clavar una lista que habría que actualizar.
    #[test]
    fn el_fondo_afin_al_acento_sale_el_primero() {
        let mut fondos = crate::fondos::instaladas(false);
        if fondos.len() < 2 {
            return;
        }
        for acento in [
            Acento::Verde,
            Acento::Rojo,
            Acento::Morado,
            Acento::Azul,
            Acento::Arena,
        ] {
            let mut orden = fondos.clone();
            ordenar_por_afinidad(&mut orden, acento);
            let objetivo = tono_de_acento(acento).expect("estos cuatro tienen tono");
            let d = |f: &crate::fondos::Familia| {
                crate::fondos::tono(f).map(|t| crate::fondos::distancia_de_tono(t, objetivo))
            };
            let primera = d(&orden[0]).expect("los fondos de BookOS traen SVG");
            for otra in &orden[1..] {
                if let Some(x) = d(otra) {
                    assert!(
                        primera <= x,
                        "con {acento:?} sale {} ({primera:.0}°) por delante de {} ({x:.0}°)",
                        orden[0].nombre,
                        otra.nombre
                    );
                }
            }
        }

        // Grafito es gris: no hay fondo que le pegue más que otro, así que se
        // queda el orden alfabético que trae `instaladas`.
        let alfabetico: Vec<String> = fondos.iter().map(|f| f.nombre.clone()).collect();
        ordenar_por_afinidad(&mut fondos, Acento::Grafito);
        let despues: Vec<String> = fondos.iter().map(|f| f.nombre.clone()).collect();
        assert_eq!(alfabetico, despues, "el gris no debería reordenar nada");
    }

    #[test]
    fn la_fila_de_fondos_elige_familia() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        if a.fondos.is_empty() {
            assert_eq!(a.fondo_en(MARGEN + 1.0, a.y_fondos() + 1.0), None);
            return;
        }
        let (paso_x, paso_y) = (ancho_fondo() + FONDO_HUECO, alto_fondo() + FONDO_HUECO);
        for i in 0..a.fondos.len() {
            let (col, fila) = (i % FONDO_COLUMNAS, i / FONDO_COLUMNAS);
            let x = MARGEN + col as f32 * paso_x + 1.0;
            let y = a.y_fondos() + fila as f32 * paso_y + 1.0;
            assert_eq!(a.fondo_en(x, y), Some(i), "la miniatura {i} no responde");
            // Pulsar pide la pareja **entera**, que es lo que permite que el
            // cambio de tema se lleve el fondo con él.
            let esperada = a.fondos[i].clone();
            assert_eq!(
                a.pulsar(x, y),
                Some(Accion::Fondo {
                    claro: esperada.claro,
                    oscuro: esperada.oscuro,
                })
            );
            assert_eq!(a.marca_fondo.actual(), Some(i));
        }
        // Las calles entre dos no son de nadie, ni la zona de debajo de la
        // última fila.
        let y = a.y_fondos() + 1.0;
        if a.fondos.len() > 1 {
            let calle = MARGEN + ancho_fondo() + FONDO_HUECO / 2.0;
            assert_eq!(a.fondo_en(calle, y), None);
        }
        let fin = a.y_fondos() + alto_fondos(a.fondos.len());
        assert_eq!(a.fondo_en(MARGEN + 1.0, fin + 5.0), None);
        // Y la rejilla entra en la tarjeta: con el alto mal contado, la última
        // fila quedaría cortada por abajo.
        assert!(fin <= a.size().1 - MARGEN + 0.01);
    }

    /// Las tres muestras eligen su modo, cada una en su sitio, y las calles
    /// que las separan no eligen nada.
    #[test]
    fn las_tres_muestras_eligen_modo() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        let y = a.y_muestras() + 4.0;
        let paso = ancho_muestra() + MUESTRA_HUECO;
        for (i, esperado) in MODOS.iter().enumerate() {
            let x = MARGEN + i as f32 * paso + 4.0;
            assert_eq!(
                a.pulsar(x, y),
                Some(Accion::Apariencia {
                    modo: *esperado,
                    acento: tema::acento_actual(),
                }),
                "la muestra {i} no eligió {esperado:?}"
            );
            // Y la calle que hay a su derecha no es de nadie: pulsar entre dos
            // muestras no puede elegir la de la izquierda.
            if i + 1 < MODOS.len() {
                let calle = MARGEN + i as f32 * paso + ancho_muestra() + MUESTRA_HUECO / 2.0;
                assert_eq!(a.muestra_en(calle, y), None, "la calle {i} eligió algo");
            }
        }
        // Y la última muestra llega hasta el borde del margen, sin sobrar ni
        // faltar: con el ancho mal repartido, la de la derecha quedaría cortada.
        assert_eq!(a.muestra_en(ANCHO - MARGEN - 1.0, y), Some(MODOS.len() - 1));
    }
}
