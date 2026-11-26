//! Los dos conmutadores de BookOS.
//!
//! Toma la forma del de macOS: una fila de iconos centrada en la pantalla, con
//! la elegida realzada y su nombre debajo. Se mantiene mientras el modificador
//! siga pulsado y se resuelve al soltarlo.
//!
//! Los dos enseñan **una celda por ventana**; lo que cambia es qué se dibuja en
//! ella. `Alt+Tab` pone el icono de la aplicación y el título debajo. `Meta+Tab`
//! deja un hueco transparente: el compositor coloca ahí la superficie viva del
//! cliente, escalada por la GPU.
//!
//! El orden es de **uso reciente**: la primera es la que acabas de dejar, así
//! que un Alt+Tab suelto alterna entre las dos últimas, que es el 90 % de los
//! usos. Ese orden lo lleva el compositor, que es quien ve el foco; aquí solo se
//! dibuja lo que llega.
//!
//! El shell **no sabe a qué ventana lleva cada celda**: devuelve el índice de la
//! elegida y el compositor lo traduce. Antes devolvía el `app_id`, y eso era lo
//! que ataba una celda a una aplicación entera.

use iced_core::alignment::Horizontal;
use iced_core::{Border, Length, Rectangle};
use iced_widget::{Column, Row, Space, column, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

/// Lado del icono de cada aplicación.
///
/// 64 es el tamaño al que los temas traen dibujo de verdad —por debajo se
/// rasteriza un SVG y por encima se interpola— y el mismo que usa el launchpad.
const ICONO: f32 = 64.0;
/// Aire alrededor del icono dentro de su celda.
const AIRE: f32 = 10.0;
/// Lado de la celda, que es lo que se realza al seleccionar.
const CELDA: f32 = ICONO + AIRE * 2.0;
/// Margen de la tarjeta.
const MARGEN: f32 = 16.0;
/// Alto del renglón del nombre, debajo de la fila.
const NOMBRE: f32 = 26.0;
/// El renglón de Meta+Tab: icono grande, aplicación y título. Con 18 px de icono
/// y una línea no se sabía de un vistazo a qué aplicación se iba a saltar.
// añadido en la 0.2.0
const ROTULO: f32 = 56.0;
const ROTULO_ICONO: f32 = 44.0;
const ROTULO_SEP: f32 = 12.0;

/// Cuántas aplicaciones caben antes de dejar de crecer.
///
/// Doce celdas son 1.008 px, que a 1.280 lógicos ya roza los bordes. Pasado ese
/// número el conmutador dejaría de caber en pantalla, y una fila que se sale es
/// peor que una lista recortada: al menos así se ve que hay más.
pub const MAXIMO: usize = 12;

/// La tarjeta de los dos conmutadores. Flota sobre cualquier ventana y, en
/// claro, blanca y translúcida sobre una ventana blanca no tenía borde visible
/// y dejaba leer el texto de debajo. Opaca, en claro con el gris de fondo del
/// sistema, y con un filo de 1 px: la sombra no cabe, el búfer mide la tarjeta.
// modificado en la 0.2.3
fn estilo_tarjeta() -> container::Style {
    let fondo = if tema::es_claro() {
        tema::bg()
    } else {
        tema::card()
    };
    let filo = if tema::alto_contraste() { 1.0 } else { 0.22 };
    container::Style {
        background: Some(fondo.into()),
        border: Border {
            radius: tema::R_TARJETA.into(),
            width: 1.0,
            color: tema::mezclar(fondo, tema::texto(), filo),
        },
        ..Default::default()
    }
}

/// Qué presenta el conmutador y, por tanto, qué significa cada celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Aplicaciones,
    Ventanas,
}

const PREVIA_ANCHO_MAX: f32 = 300.0;
const PREVIA_ALTO_MAX: f32 = 250.0;
const PREVIA_HUECO: f32 = 16.0;
const PREVIA_MARGEN_PANTALLA: f32 = 24.0;
/// Hasta qué parte de la pantalla puede crecer una miniatura.
// añadido en la 0.2.0
const PREVIA_TOPE_ANCHO: f32 = 0.56;
const PREVIA_TOPE_ALTO: f32 = 0.68;
/// Lo que se deja libre arriba y abajo para el panel y el dock. La tarjeta va
/// centrada, así que manda el lado mayor —el dock, con su margen—: con la
/// rejilla más grande, sin esto el rótulo se montaba sobre los iconos del dock
/// (visto en el anidado).
// añadido en la 0.2.0
const PREVIA_RESERVA_ALTO: f32 = 64.0;
/// Aire entre el borde de la celda y la ventana viva que se compone dentro.
///
/// 6 px con radio exterior `R_POPOVER` (18) y radio interior `R_BOTON` (12)
/// mantiene esquinas concéntricas exactas (18 - 6 = 12) y evita que la ventana
/// con esquinas rectas tape el radio de la celda.
const PREVIA_BORDE: f32 = 6.0;

/// Una celda de la fila: una ventana.
pub struct Entrada {
    /// Lo que se enseña debajo cuando está elegida: el título de la ventana.
    pub nombre: String,
    /// La aplicación, para el rótulo de Meta+Tab: el título solo no siempre la
    /// dice («Sin título», el nombre de un documento).
    pub app: String,
    /// El de su aplicación. Dos ventanas del mismo programa repiten icono, y por
    /// eso el que las distingue es el nombre.
    pub icono: Option<Icono>,
}

pub struct Conmutador {
    modo: Modo,
    entradas: Vec<Entrada>,
    elegida: usize,
    /// El recuadro de la elegida, moviéndose de una celda a otra.
    ///
    /// Va aparte de `elegida` porque quien decide qué está elegido es el
    /// teclado y tiene que saberlo al instante —soltar Alt activa esa ventana—
    /// mientras que el recuadro puede ir un fotograma por detrás.
    marca: tema::Realce,
    rejilla: Option<Rejilla>,
}

#[derive(Debug, Clone, Copy)]
struct Rejilla {
    columnas: usize,
    filas: usize,
    celda_ancho: f32,
    celda_alto: f32,
}

impl Conmutador {
    // 0.1.0
    pub fn new(modo: Modo, entradas: Vec<Entrada>, pantalla: (f32, f32)) -> Self {
        // La primera es la que está enfocada. El compositor aplica después el
        // primer paso (+1 o -1), igual que los pasos repetidos; así Mayús no
        // necesita una excepción distinta al abrir.
        let elegida = 0;
        let rejilla = (modo == Modo::Ventanas).then(|| Rejilla::new(entradas.len(), pantalla));
        let mut marca = tema::Realce::nuevo();
        // Ya puesta en la primera al abrir: la tarjeta aparece con su recuadro,
        // no dibujándolo.
        marca.señalar(Some(elegida));
        marca.terminar();
        Self {
            modo,
            entradas,
            elegida,
            marca,
            rejilla,
        }
    }

    // 0.1.0
    pub fn vacio(&self) -> bool {
        self.entradas.is_empty()
    }

    /// El índice de la elegida, que es lo que el compositor traduce a ventana.
    // 0.1.0
    pub fn elegida(&self) -> Option<usize> {
        (self.elegida < self.entradas.len()).then_some(self.elegida)
    }

    /// Avanza la selección. `1` hacia delante, `-1` hacia atrás con Mayús.
    ///
    /// Aquí **sí** se da la vuelta, al contrario que en los escritorios: la fila
    /// se ve entera, así que pasar del último al primero no desorienta —lo
    /// tienes delante— y recorrer ocho aplicaciones sin poder volver por el otro
    /// lado sería absurdo.
    // 0.1.0
    pub fn mover(&mut self, pasos: i32) {
        if self.entradas.is_empty() {
            return;
        }
        let n = self.entradas.len() as isize;
        self.elegida = (self.elegida as isize + pasos as isize).rem_euclid(n) as usize;
        self.marca.señalar(Some(self.elegida));
    }

    /// ¿Se está moviendo el recuadro? Mientras sí, hay que repintar.
    ///
    /// En el de ventanas nunca: ahí el recuadro es una superficie aparte que el
    /// compositor desliza sin repintar nada. Repintar la tarjeta en cada
    /// fotograma del cambio era el tirón al pasar el cursor por las miniaturas:
    /// medido en release a 2880 px y escala 1,75, **8,3 ms con tres ventanas,
    /// 11,4 con seis y 25,4 con diez**, por fotograma, con 8,3 de presupuesto a
    /// 120 Hz (`examples/conmutador_bench.rs`).
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.rejilla.is_none() && self.marca.animando()
    }

    /// La celda de la elegida, en lógicos de la tarjeta. Solo en el de
    /// ventanas, que es el que lleva el recuadro aparte.
    // 0.1.0
    pub fn realce(&self) -> Option<Rectangle> {
        let r = self.rejilla?;
        (self.elegida < self.entradas.len()).then(|| r.celda(self.elegida, self.entradas.len()))
    }

    /// La franja del rótulo —icono y título de la elegida— en lógicos de la
    /// tarjeta. Va aparte por lo mismo que el recuadro: es lo único de la
    /// tarjeta que cambia al elegir, y mide un renglón.
    // 0.1.0
    pub fn rotulo(&self) -> Option<Rectangle> {
        let r = self.rejilla?;
        let (w, h) = r.size();
        Some(Rectangle {
            x: MARGEN,
            y: MARGEN + h,
            width: w,
            height: ROTULO,
        })
    }

    /// El recuadro de la elegida, solo, para su propia superficie.
    ///
    /// Se compone **encima** de la celda de la tarjeta, que lleva ya su fondo y
    /// su borde de reposo: por eso el relleno es el acento translúcido y el
    /// borde el acento entero, los mismos que llevaba la celda elegida.
    // 0.1.0
    pub fn view_realce() -> PanelElement<'static> {
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::alfa(tema::acento(), 0.18).into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: if tema::alto_contraste() { 3.0 } else { 2.5 },
                    color: tema::acento(),
                },
                ..Default::default()
            })
            .into()
    }

    /// El rótulo de la elegida: su icono y su título. El icono aquí y no en la
    /// celda porque dos ventanas de la misma terminal son idénticas a este
    /// tamaño, y hace falta ver de qué programa es la que vas a sacar.
    // 0.1.0
    pub fn view_rotulo(&self) -> PanelElement<'_> {
        let elegida = self.entradas.get(self.elegida);
        let mut rotulo = Row::new()
            .spacing(ROTULO_SEP)
            .align_y(iced_core::Alignment::Center);
        if let Some(ic) = elegida.and_then(|e| e.icono.as_ref()) {
            rotulo = rotulo.push(icono::ver(ic, ROTULO_ICONO, ROTULO_ICONO));
        }
        // Sin recortar, un título largo empujaba el icono fuera del renglón.
        let ancho = (self.rotulo().map_or(0.0, |r| r.width) - ROTULO_ICONO - ROTULO_SEP).max(0.0);
        let linea =
            |texto: &str, tamaño: f32| crate::emergente::lista::recortar(texto, ancho, tamaño);
        let (app, titulo) = elegida.map_or((String::new(), String::new()), |e| {
            (
                linea(&e.app, tema::T_TITULO),
                linea(&e.nombre, tema::T_CUERPO),
            )
        });
        let mut textos = Column::new().spacing(2).push(
            text(app.clone())
                .size(tema::T_TITULO)
                .font(iced_core::Font {
                    weight: iced_core::font::Weight::Semibold,
                    ..iced_core::Font::DEFAULT
                })
                .color(tema::texto())
                .wrapping(iced_core::text::Wrapping::None),
        );
        // El título repetido no dice nada: una ventana sin título propio lo
        // hereda de la aplicación.
        if titulo != app {
            textos = textos.push(
                text(titulo)
                    .size(tema::T_CUERPO)
                    .color(tema::texto2())
                    .wrapping(iced_core::text::Wrapping::None),
            );
        }
        rotulo = rotulo.push(textos);
        container(rotulo)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }

    /// Qué celda cae en un punto, para poder elegir con el ratón.
    // 0.1.0
    pub fn en(&self, x: f32, y: f32) -> Option<usize> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        match self.rejilla {
            None => {
                if x < MARGEN || y < MARGEN || y >= MARGEN + CELDA {
                    return None;
                }
                let i = ((x - MARGEN) / CELDA) as usize;
                (i < self.entradas.len()).then_some(i)
            }
            Some(r) => r.en(x, y, self.entradas.len()),
        }
    }

    // 0.1.0
    pub fn elegir(&mut self, i: usize) -> bool {
        if i < self.entradas.len() && i != self.elegida {
            self.elegida = i;
            self.marca.señalar(Some(i));
            true
        } else {
            false
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        match self.rejilla {
            // La tarjeta de ventanas se mide como la de aplicaciones: celdas,
            // renglón del nombre y margen alrededor.
            Some(r) => {
                let (w, h) = r.size();
                (w + MARGEN * 2.0, h + ROTULO + MARGEN * 2.0)
            }
            None => {
                let n = self.entradas.len().max(1) as f32;
                (n * CELDA + MARGEN * 2.0, CELDA + NOMBRE + MARGEN * 2.0)
            }
        }
    }

    /// Huecos transparentes en los que el compositor coloca cada ventana.
    // 0.1.0
    pub fn miniaturas(&self) -> Vec<Rectangle> {
        let Some(r) = self.rejilla else {
            return Vec::new();
        };
        let n = self.entradas.len();
        (0..n).map(|i| r.miniatura(i, n)).collect()
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        if self.modo == Modo::Ventanas {
            return self.view_ventanas();
        }

        let mut fila = row![];
        for (i, entrada) in self.entradas.iter().enumerate() {
            let elegida = self.marca.intensidad(i);
            let dibujo: PanelElement<'_> = match &entrada.icono {
                // Sin teñir: son iconos de aplicación, y convertir el zorro de
                // Firefox en una silueta blanca no ayuda a reconocerlo.
                Some(ic) => icono::ver(ic, ICONO, ICONO),
                None => Space::new().width(Length::Fixed(ICONO)).into(),
            };
            fila = fila.push(
                container(dibujo)
                    .width(Length::Fixed(CELDA))
                    .height(Length::Fixed(CELDA))
                    .center_x(Length::Fixed(CELDA))
                    .center_y(Length::Fixed(CELDA))
                    .style(move |_theme: &iced_widget::Theme| container::Style {
                        // La elegida lleva un recuadro por detrás, como en
                        // macOS. Nada de un borde fino a secas: sobre iconos de
                        // colores un contorno se pierde y el relleno se ve de
                        // refilón. El relleno es el acento apagado —el azul es
                        // lo que significa «esto está elegido» en el resto del
                        // sistema— y encima va un contorno del acento entero,
                        // que es lo que lo separa de un icono azul cualquiera.
                        // El recuadro no salta de una celda a otra: se apaga en
                        // la que deja y se enciende en la que toma, con el
                        // token de hover. Alt+Tab repetido recorre la fila y el
                        // salto seco se leía como un parpadeo por icono.
                        background: Some(tema::alfa(tema::acento(), 0.22 * elegida).into()),
                        border: Border {
                            radius: tema::R_CONTROL.into(),
                            width: 1.5 * elegida,
                            color: tema::alfa(tema::acento(), elegida),
                        },
                        ..Default::default()
                    }),
            );
        }

        let nombre = self
            .entradas
            .get(self.elegida)
            .map(|e| e.nombre.clone())
            .unwrap_or_default();
        let contenido = column![
            fila,
            container(text(nombre).size(tema::T_CUERPO).color(tema::texto()))
                .width(Length::Fill)
                .height(Length::Fixed(NOMBRE))
                .align_x(Horizontal::Center)
                .center_y(Length::Fixed(NOMBRE)),
        ]
        .align_x(Horizontal::Center);

        container(contenido)
            .padding(MARGEN)
            .style(|_theme: &iced_widget::Theme| estilo_tarjeta())
            .into()
    }

    /// Meta+Tab: la misma tarjeta que Alt+Tab, con miniaturas en vez de iconos.
    ///
    /// El nombre va **una sola vez** debajo de la fila, como en el conmutador de
    /// macOS y como en el modo de aplicaciones de aquí al lado. Antes cada celda
    /// llevaba el suyo: con cuatro terminales abiertas eran cuatro títulos
    /// largos compitiendo, y el que importa es el de la que vas a elegir.
    // 0.1.0
    fn view_ventanas(&self) -> PanelElement<'_> {
        let r = self
            .rejilla
            .expect("el modo ventanas siempre tiene rejilla");
        let mut rejilla = Column::new()
            .spacing(PREVIA_HUECO)
            .align_x(Horizontal::Center);
        for fila_i in 0..r.filas {
            let mut fila = Row::new().spacing(PREVIA_HUECO);
            for columna in 0..r.columnas {
                let i = fila_i * r.columnas + columna;
                if self.entradas.get(i).is_some() {
                    // Todas en reposo: la elegida la marca el recuadro, que es
                    // otra superficie. Así elegir no repinta la tarjeta.
                    let elegida = 0.0;
                    // La celda va vacía a propósito: lo que se ve dentro es la
                    // ventana del cliente, que el compositor compone encima. Un
                    // icono aquí quedaría medio tapado por ella —se probó, se ve
                    // asomar una esquina— y por eso el icono va con el nombre.
                    fila = fila.push(
                        container(Space::new())
                            .width(Length::Fixed(r.celda_ancho))
                            .height(Length::Fixed(r.celda_alto))
                            .style(move |_theme: &iced_widget::Theme| container::Style {
                                // La celda tiene fondo: la ventana viva se compone
                                // **encima**, así que el relleno solo se ve en el
                                // marco, y sin él la celda era un marco flotando.
                                background: Some(
                                    tema::mezclar(
                                        tema::alfa(tema::tinta(), 0.07),
                                        tema::alfa(tema::acento(), 0.18),
                                        elegida,
                                    )
                                    .into(),
                                ),
                                border: Border {
                                    radius: tema::R_POPOVER.into(),
                                    // El marco de la elegida es más grueso: crece
                                    // con ella en vez de aparecer de golpe.
                                    width: 1.0 + 1.5 * elegida,
                                    color: tema::mezclar(tema::borde(), tema::acento(), elegida),
                                },
                                ..Default::default()
                            }),
                    );
                }
            }
            rejilla = rejilla.push(fila);
        }

        // El renglón del rótulo se queda vacío: lo pone `view_rotulo` en su
        // propia superficie, encima.
        let contenido = column![rejilla, Space::new().height(Length::Fixed(ROTULO))]
            .align_x(Horizontal::Center);

        container(contenido)
            .padding(MARGEN)
            .style(|_theme: &iced_widget::Theme| estilo_tarjeta())
            .into()
    }
}

impl Rejilla {
    /// Una fila mientras las miniaturas salgan de un tamaño reconocible, y
    /// varias solo cuando no queda sitio.
    ///
    /// La fila única es la forma del conmutador de macOS y es la que se lee de
    /// un vistazo: el recorrido con Tab es horizontal, y una rejilla obliga a
    /// buscar dónde saltó la selección al cambiar de renglón. Con doce ventanas
    /// en una pantalla pequeña deja de caber, y entonces sí se parte.
    // 0.1.0
    fn new(n: usize, pantalla: (f32, f32)) -> Self {
        let pantalla = (
            if pantalla.0.is_finite() { pantalla.0 } else { 1280.0 },
            if pantalla.1.is_finite() { pantalla.1 } else { 720.0 },
        );
        let n = n.max(1);
        let ancho = (pantalla.0 - (PREVIA_MARGEN_PANTALLA + MARGEN) * 2.0).max(160.0);
        let alto = (pantalla.1 - (PREVIA_RESERVA_ALTO + MARGEN) * 2.0 - ROTULO).max(120.0);
        // Las miniaturas llevan la proporción de la pantalla: lo que se compone
        // dentro es una ventana de ese escritorio, y otra proporción la dejaría
        // con franjas a los lados.
        let proporcion = (pantalla.1 / pantalla.0.max(1.0)).clamp(0.4, 1.4);

        // Los topes siguen a la pantalla: 300×250 fijos eran el 21 % del ancho
        // de un portátil de 1440 y se quedaban pequeños en cualquier pantalla
        // mayor, con la mayor parte del escritorio vacía a su alrededor.
        let ancho_max = (pantalla.0 * PREVIA_TOPE_ANCHO).max(PREVIA_ANCHO_MAX);
        let alto_max = (pantalla.1 * PREVIA_TOPE_ALTO).max(PREVIA_ALTO_MAX);

        // Qué mide una celda con `columnas` columnas: primero lo que da de sí
        // el ancho, luego se recorta por el alto que queda, y el ancho se
        // recorta con él para que la celda no sea un apaisado con la ventana
        // flotando dentro.
        let medidas = |columnas: usize| -> (f32, f32) {
            let filas = n.div_ceil(columnas);
            let celda_ancho = ((ancho - PREVIA_HUECO * (columnas - 1) as f32) / columnas as f32)
                .clamp(36.0, ancho_max);
            let cabe_de_alto = (alto - PREVIA_HUECO * (filas - 1) as f32) / filas as f32;
            let celda_alto = (celda_ancho * proporcion)
                .min(cabe_de_alto)
                .clamp(28.0, alto_max);
            (
                celda_ancho.min(celda_alto / proporcion.max(0.01)),
                celda_alto,
            )
        };

        // Hasta 5 ventanas van en una sola fila. A partir de 6 se reparten en
        // filas equilibradas de como mucho 4 columnas (6 → 3+3, 7 → 4+3,
        // 8 → 4+4, 9 → 3+3+3, 10 → 4+4+2, 11 → 4+4+3, 12 → 4+4+4) evitando
        // tanto filas de 5 en rejilla como dejar 1 ventana suelta abajo.
        // Si con muchas ventanas (p. ej. 48) no cupieran de alto, se amplían
        // las columnas lo justo para no salirse de la pantalla.
        let columnas = if n <= 5 {
            n
        } else {
            let filas_min = n.div_ceil(4).max(2);
            let pref = n.div_ceil(filas_min);
            let cabe_alto = |c: usize| {
                let f = n.div_ceil(c);
                let (_, ch) = medidas(c);
                f as f32 * ch + (f - 1) as f32 * PREVIA_HUECO <= alto + 0.1
            };
            if cabe_alto(pref) {
                pref
            } else {
                (pref..=n)
                    .find(|&c| cabe_alto(c))
                    .unwrap_or_else(|| {
                        (1..=n)
                            .max_by(|a, b| medidas(*a).0.total_cmp(&medidas(*b).0))
                            .unwrap_or(1)
                    })
            }
        };
        let filas = n.div_ceil(columnas);
        let (celda_ancho, celda_alto) = medidas(columnas);
        Self {
            columnas,
            filas,
            celda_ancho,
            celda_alto,
        }
    }

    /// Lo que ocupan las celdas, sin el margen de la tarjeta ni el rótulo.
    // 0.1.0
    fn size(self) -> (f32, f32) {
        (
            self.columnas as f32 * self.celda_ancho
                + self.columnas.saturating_sub(1) as f32 * PREVIA_HUECO,
            self.filas as f32 * self.celda_alto
                + self.filas.saturating_sub(1) as f32 * PREVIA_HUECO,
        )
    }

    fn sangria(self, fila: usize, n: usize) -> f32 {
        let inicio = fila * self.columnas;
        let en_fila = n.saturating_sub(inicio).min(self.columnas);
        let faltan = self.columnas.saturating_sub(en_fila);
        faltan as f32 * (self.celda_ancho + PREVIA_HUECO) * 0.5
    }

    /// La celda entera, marco incluido.
    // 0.1.0
    fn celda(self, i: usize, n: usize) -> Rectangle {
        let columna = i % self.columnas;
        let fila = i / self.columnas;
        Rectangle {
            x: MARGEN + self.sangria(fila, n) + columna as f32 * (self.celda_ancho + PREVIA_HUECO),
            y: MARGEN + fila as f32 * (self.celda_alto + PREVIA_HUECO),
            width: self.celda_ancho,
            height: self.celda_alto,
        }
    }

    // 0.1.0
    fn miniatura(self, i: usize, n: usize) -> Rectangle {
        let columna = i % self.columnas;
        let fila = i / self.columnas;
        Rectangle {
            x: MARGEN
                + self.sangria(fila, n)
                + columna as f32 * (self.celda_ancho + PREVIA_HUECO)
                + PREVIA_BORDE,
            y: MARGEN + fila as f32 * (self.celda_alto + PREVIA_HUECO) + PREVIA_BORDE,
            width: (self.celda_ancho - PREVIA_BORDE * 2.0).max(1.0),
            height: (self.celda_alto - PREVIA_BORDE * 2.0).max(1.0),
        }
    }

    // 0.1.0
    fn en(self, x: f32, y: f32, n: usize) -> Option<usize> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (x, y) = (x - MARGEN, y - MARGEN);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let paso_x = self.celda_ancho + PREVIA_HUECO;
        let paso_y = self.celda_alto + PREVIA_HUECO;
        let fila = (y / paso_y) as usize;
        if fila >= self.filas {
            return None;
        }
        let x_fila = x - self.sangria(fila, n);
        if x_fila < 0.0 {
            return None;
        }
        let columna = (x_fila / paso_x) as usize;
        if columna >= self.columnas {
            return None;
        }
        let dentro_x = x_fila - columna as f32 * paso_x < self.celda_ancho;
        let dentro_y = y - fila as f32 * paso_y < self.celda_alto;
        let i = fila * self.columnas + columna;
        (dentro_x && dentro_y && i < n).then_some(i)
    }
}

const FUENTE_TACTIL: iced_core::Font = iced_core::Font {
    weight: iced_core::font::Weight::Semibold,
    ..iced_core::Font::DEFAULT
};
const ICONO_TACTIL: f32 = 24.0;
const SEP_TACTIL: f32 = 8.0;
const MARGEN_TACTIL: f32 = 16.0;

pub fn ancho_etiqueta_tactil(entradas: &[Entrada]) -> f32 {
    entradas
        .iter()
        .map(|e| {
            crate::widget::ancho_con_fuente(&e.nombre, tema::T_CUERPO, FUENTE_TACTIL)
                + e.icono.as_ref().map_or(0.0, |_| ICONO_TACTIL + SEP_TACTIL)
        })
        .sum::<f32>()
        + 2.0 * MARGEN_TACTIL
        + SEP_TACTIL * entradas.len().saturating_sub(1) as f32
}

pub(crate) fn etiqueta_tactil(entradas: &[Entrada], ancho: f32, foco: bool) -> PanelElement<'_> {
    let fondo = tema::card();
    let tinta = tema::texto();
    let mut contenido = Row::new()
        .spacing(SEP_TACTIL)
        .align_y(iced_core::Alignment::Center);
    let libre = ancho - 2.0 * MARGEN_TACTIL - SEP_TACTIL * entradas.len().saturating_sub(1) as f32;
    let ajustar = ancho_etiqueta_tactil(entradas) > ancho;
    let hueco = libre / entradas.len().max(1) as f32;
    for e in entradas {
        let ancho_icono = e.icono.as_ref().map_or(0.0, |_| ICONO_TACTIL + SEP_TACTIL);
        if let Some(icono) = &e.icono {
            contenido = contenido.push(icono::ver(icono, ICONO_TACTIL, ICONO_TACTIL));
        }
        let nombre = if ajustar {
            std::borrow::Cow::Owned(crate::emergente::lista::recortar(
                &e.nombre,
                (hueco - ancho_icono).max(0.0),
                tema::T_CUERPO,
            ))
        } else {
            std::borrow::Cow::Borrowed(e.nombre.as_str())
        };
        contenido = contenido.push(
            text(nombre)
                .size(tema::T_CUERPO)
                .font(FUENTE_TACTIL)
                .color(tinta)
                .wrapping(iced_core::text::Wrapping::None),
        );
    }
    container(contenido)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_POPOVER.into(),
                width: if foco || tema::alto_contraste() {
                    2.0
                } else {
                    0.0
                },
                color: tinta,
            },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entradas(n: usize) -> Vec<Entrada> {
        (0..n)
            .map(|i| Entrada {
                nombre: format!("Ventana {i}"),
                app: "Aplicación".into(),
                icono: None,
            })
            .collect()
    }

    /// Con varias ventanas, el conmutador arranca en la **segunda**: quien pulsa
    /// Alt+Tab quiere la de antes, no la que ya tiene delante.
    #[test]
    fn arranca_en_la_actual_y_el_primer_paso_va_a_la_anterior() {
        let c = Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0));
        assert_eq!(c.elegida(), Some(0));
        let mut c = c;
        c.mover(1);
        assert_eq!(c.elegida(), Some(1));
    }

    /// Con una sola no hay a dónde ir.
    #[test]
    fn con_una_sola_se_queda_donde_esta() {
        let c = Conmutador::new(Modo::Aplicaciones, entradas(1), (1280.0, 720.0));
        assert_eq!(c.elegida(), Some(0));
    }

    /// La fila **sí** da la vuelta, al revés que los escritorios: se ve entera,
    /// así que pasar del último al primero no desorienta.
    #[test]
    fn la_seleccion_da_la_vuelta() {
        let mut c = Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0));
        c.mover(1);
        assert_eq!(c.elegida(), Some(1));
        c.mover(1);
        assert_eq!(c.elegida(), Some(2));
        c.mover(1);
        assert_eq!(c.elegida(), Some(0), "del último al primero");
        c.mover(-1);
        assert_eq!(c.elegida(), Some(2), "y hacia atrás con Mayús");
    }

    /// El ancho crece con las aplicaciones: es lo que usa el compositor para
    /// centrar la tarjeta, así que si miente, sale descentrada.
    #[test]
    fn el_ancho_cuenta_las_celdas() {
        assert_eq!(
            Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0))
                .size()
                .0,
            3.0 * CELDA + MARGEN * 2.0
        );
    }

    /// Elegir con el ratón acierta la celda de debajo del cursor.
    #[test]
    fn el_raton_elige_su_celda() {
        let c = Conmutador::new(Modo::Aplicaciones, entradas(4), (1280.0, 720.0));
        assert_eq!(c.en(MARGEN + 1.0, 40.0), Some(0));
        assert_eq!(c.en(MARGEN + CELDA * 2.5, 40.0), Some(2));
        assert_eq!(c.en(MARGEN + CELDA * 9.0, 40.0), None, "fuera de la fila");
    }

    #[test]
    fn la_rejilla_da_un_hueco_y_hit_test_por_ventana() {
        let c = Conmutador::new(Modo::Ventanas, entradas(5), (1280.0, 720.0));
        let miniaturas = c.miniaturas();
        assert_eq!(miniaturas.len(), 5);
        let primera = miniaturas[0];
        assert_eq!(
            c.en(
                primera.x + primera.width / 2.0,
                primera.y + primera.height / 2.0
            ),
            Some(0)
        );
        let (w, h) = c.size();
        assert!(w <= 1280.0 && h <= 720.0);
    }

    #[test]
    fn la_rejilla_equilibra_filas_sin_dejar_una_suelta() {
        assert_eq!(Rejilla::new(4, (1280.0, 720.0)).columnas, 4);
        assert_eq!(Rejilla::new(5, (1280.0, 720.0)).columnas, 5);
        assert_eq!(Rejilla::new(6, (1280.0, 720.0)).columnas, 3);
        assert_eq!(Rejilla::new(7, (1280.0, 720.0)).columnas, 4);
        assert_eq!(Rejilla::new(8, (1280.0, 720.0)).columnas, 4);
        assert_eq!(Rejilla::new(9, (1280.0, 720.0)).columnas, 3);
        assert_eq!(Rejilla::new(10, (1280.0, 720.0)).columnas, 4);
        assert_eq!(Rejilla::new(12, (1280.0, 720.0)).columnas, 4);
    }

    #[test]
    fn muchas_ventanas_siguen_dentro_de_la_pantalla() {
        let c = Conmutador::new(Modo::Ventanas, entradas(48), (1280.0, 720.0));
        assert_eq!(c.miniaturas().len(), 48);
        let (w, h) = c.size();
        assert!(w <= 1280.0 - PREVIA_MARGEN_PANTALLA * 2.0 + 0.1);
        assert!(h <= 720.0 - PREVIA_MARGEN_PANTALLA * 2.0 + 0.1);
    }

    #[test]
    fn mover_extremos_y_coordenadas_o_pantalla_no_finita_son_seguros() {
        let mut c = Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0));
        c.mover(i32::MAX);
        assert!(c.elegida().is_some_and(|i| i < 3));
        c.mover(i32::MIN);
        assert!(c.elegida().is_some_and(|i| i < 3));
        assert_eq!(c.en(f32::NAN, 40.0), None);
        assert_eq!(c.en(MARGEN + 5.0, f32::INFINITY), None);

        let v = Conmutador::new(Modo::Ventanas, entradas(4), (f32::NAN, f32::NAN));
        let (w, h) = v.size();
        assert!(w.is_finite() && w > 0.0);
        assert!(h.is_finite() && h > 0.0);
        assert_eq!(v.en(f32::NAN, 40.0), None);
    }
}
