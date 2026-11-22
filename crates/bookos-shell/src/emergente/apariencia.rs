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

use iced_core::{Border, Color, Length, Padding, Point, Rectangle};
use iced_widget::{Space, column, container, row, stack, text};

use crate::Accion;
use crate::tema::{self, Acento, ModoTema, Realce};
use crate::view::PanelElement;

use super::{Ancla, Tecla, t};

// Medidas de la mesa «Apariencia» del lienzo: las piezas de las demás
// tarjetas (Batería, Wi‑Fi): 336 de ancho, relleno 16, título 15/600,
// secciones en gris con separadores.
const ANCHO: f32 = 336.0;
const MARGEN: f32 = 16.0;
/// El contenido entra 4 px más por los lados, como los rótulos de Batería.
const SANGRIA: f32 = 4.0;
const TITULO: f32 = 20.0;
/// El selector de modo: el mismo que «Modo de energía». Cápsula de 40 con
/// un pulgar de 34 que se desliza.
const SELECTOR: f32 = 40.0;
const SELECTOR_RELLENO: f32 = 3.0;
/// Hueco antes de un separador y después de él hasta el rótulo.
const AIRE: f32 = 16.0;
const ROTULO: f32 = 15.0;
const ROTULO_BAJO: f32 = 10.0;
/// El grupo de acentos: relleno 14×10, círculos de 24, filas cada 38.
const GRUPO_V: f32 = 14.0;
const GRUPO_H: f32 = 10.0;
const CIRCULO: f32 = 24.0;
const FILA_ACENTO: f32 = CIRCULO + 14.0;
const COLUMNAS: usize = 6;
/// El aro de lo elegido: 2 px de aire del color de detrás y 2 del anillo.
const ARO: f32 = 4.0;
/// Fondos: tres columnas con 8 de calle; la última casilla libre es «+».
const FONDO_COLUMNAS: usize = 3;
const FONDO_HUECO: f32 = 8.0;
const FONDO_RADIO: f32 = tema::R_BOTON_PEQUENO;

/// Los tres modos, en el orden de macOS e iOS: el automático, que es el de
/// serie, primero.
const MODOS: [ModoTema; 3] = [ModoTema::Automatico, ModoTema::Claro, ModoTema::Oscuro];

// añadido en la 0.2.2
fn ancho_util() -> f32 {
    ANCHO - 2.0 * MARGEN
}

/// Una casilla de la rejilla de fondos, con el aro incluido.
// añadido en la 0.2.2
fn celda_fondo() -> (f32, f32) {
    let w = (ancho_util() - FONDO_HUECO * (FONDO_COLUMNAS - 1) as f32) / FONDO_COLUMNAS as f32;
    (w, ((w - 2.0 * ARO) * 5.0 / 8.0).round() + 2.0 * ARO)
}

// 0.1.0
fn filas() -> usize {
    Acento::TODOS.len().div_ceil(COLUMNAS)
}

/// Qué tiene el foco del teclado. Tab pasa de sección en sección; las flechas
/// eligen dentro de la sección (patrón de grupo de opciones: moverse es
/// elegir, como en Batería).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Foco {
    Modo,
    Acento,
    Fondo,
    /// La casilla «+», que abre Ajustes › Fondo de pantalla.
    Mas,
}

pub struct Apariencia {
    /// Lo elegido. Es el **modo**, no el tema pintado: con «Automático»
    /// puesto, `actual()` dice claro u oscuro según la hora.
    modo: ModoTema,
    acento: Acento,
    /// El pulgar del selector de modo, de 0 a 2.
    pulgar: tema::Transicion,
    hover_modo: Realce,
    hover_color: Realce,
    marca_color: Realce,
    /// Las parejas de fondos instaladas, leídas al abrir y al cambiar de tema.
    fondos: Vec<crate::fondos::Familia>,
    /// Si `fondos` y sus miniaturas son de la variante clara. Con «Automático»
    /// la tarjeta no sabe qué toca sin la hora y la configuración; el
    /// compositor aplica el tema un fotograma después y aquí se nota en el
    /// siguiente evento, que rehace las miniaturas de la otra variante.
    fondos_claro: bool,
    vistas: Vec<Option<iced_core::image::Handle>>,
    hover_fondo: Realce,
    marca_fondo: Realce,
    /// La casilla del acento automático ya rasterizada, con la paleta de la
    /// que salió. Interior porque la paleta cambia con la tarjeta abierta y
    /// quien se entera es `view`, que solo tiene `&self`.
    franjas: std::cell::RefCell<Option<([Color; 3], iced_core::image::Handle)>>,
    foco: Option<Foco>,
}

impl Apariencia {
    /// `modo` es lo que dice la configuración, que la tarjeta no puede deducir:
    /// `tema::actual()` solo sabe qué se está pintando.
    // 0.1.0
    // modified 0.2.2 -- selector de modo, aros opacos y teclado
    pub fn new(modo: ModoTema) -> Self {
        let acento = tema::acento_actual();
        let mut marca_color = Realce::nuevo();
        marca_color.señalar(indice(acento));
        marca_color.terminar();
        let claro = tema::es_claro();
        refrescar_catalogo(claro);
        precalentar_variante(!claro);
        let (fondos, elegido) = Self::fondos_para(claro, acento);
        let mut marca_fondo = Realce::nuevo();
        marca_fondo.señalar(elegido);
        marca_fondo.terminar();
        let posicion = MODOS.iter().position(|m| *m == modo).unwrap_or(0) as f32;
        Self {
            modo,
            acento,
            pulgar: tema::Transicion::nueva(posicion, tema::M_SEGMENTADO),
            hover_modo: Realce::nuevo(),
            hover_color: Realce::nuevo(),
            marca_color,
            vistas: vistas_de(&fondos),
            fondos,
            fondos_claro: claro,
            hover_fondo: Realce::nuevo(),
            marca_fondo,
            franjas: Default::default(),
            foco: None,
        }
    }

    /// La imagen de la casilla automática, rehecha solo si cambió la paleta.
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
    /// acento, y el índice del que ya está puesto (por nombre de familia: con
    /// el tema oscuro el fichero cargado es el `_dark` de la pareja).
    // 0.1.0
    // modified 0.2.3 -- reutiliza el catálogo leído al abrir para no tocar disco al conmutar claro/oscuro
    fn fondos_para(es_claro: bool, acento: Acento) -> (Vec<crate::fondos::Familia>, Option<usize>) {
        let mut entradas = catalogo_cacheado(es_claro);
        ordenar_catalogo_por_afinidad(&mut entradas, acento);
        let fondos: Vec<crate::fondos::Familia> = entradas.into_iter().map(|(f, _)| f).collect();
        let puesta = crate::fondos::elegida();
        let elegido = puesta
            .as_deref()
            .and_then(|nombre| fondos.iter().position(|f| f.nombre == nombre));
        (fondos, elegido)
    }

    /// Rehace los fondos si el tema pintado ya no es el de sus miniaturas.
    // añadido en la 0.2.2
    fn al_dia(&mut self) -> bool {
        let claro = tema::es_claro();
        if claro == self.fondos_claro {
            return false;
        }
        let (fondos, elegido) = Self::fondos_para(claro, self.acento);
        self.vistas = vistas_de(&fondos);
        self.fondos = fondos;
        self.fondos_claro = claro;
        self.marca_fondo.señalar(elegido);
        self.marca_fondo.terminar();
        true
    }

    // --- Geometría -------------------------------------------------------------

    // añadido en la 0.2.2
    fn y_selector(&self) -> f32 {
        MARGEN + 2.0 + TITULO + 14.0
    }

    // añadido en la 0.2.2
    fn y_grupo_acento(&self) -> f32 {
        self.y_selector() + SELECTOR + AIRE + 0.5 + AIRE + ROTULO + ROTULO_BAJO
    }

    // añadido en la 0.2.2
    fn alto_grupo_acento(&self) -> f32 {
        2.0 * GRUPO_V + CIRCULO + FILA_ACENTO * (filas() - 1) as f32
    }

    // añadido en la 0.2.2
    fn y_fondos(&self) -> f32 {
        self.y_grupo_acento() + self.alto_grupo_acento() + AIRE + 0.5 + AIRE + ROTULO + ROTULO_BAJO
    }

    /// Casillas de la rejilla de fondos: las familias y el «+».
    // añadido en la 0.2.2
    fn celdas_fondo(&self) -> usize {
        self.fondos.len() + 1
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        let (_, h) = celda_fondo();
        let filas = self.celdas_fondo().div_ceil(FONDO_COLUMNAS) as f32;
        let alto = self.y_fondos() + filas * h + (filas - 1.0) * FONDO_HUECO + MARGEN + 2.0;
        (ANCHO, alto)
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        // Cuelga del panel por la izquierda, como el menú del que se abre.
        Ancla::BajoElPanel { x: 8.0 }
    }

    // añadido en la 0.2.2
    fn rect_modo(&self, i: usize) -> Rectangle {
        let w = (ancho_util() - 2.0 * SANGRIA - 2.0 * SELECTOR_RELLENO) / MODOS.len() as f32;
        Rectangle {
            x: MARGEN + SANGRIA + SELECTOR_RELLENO + i as f32 * w,
            y: self.y_selector() + SELECTOR_RELLENO,
            width: w,
            height: SELECTOR - 2.0 * SELECTOR_RELLENO,
        }
    }

    /// Cada color ocupa su sexta parte del grupo y una fila entera: zonas de
    /// 47×38, mayores que el círculo de 24 y sin calles muertas (HIG §11).
    // añadido en la 0.2.2
    fn rect_color(&self, i: usize) -> Rectangle {
        let w = (ancho_util() - 2.0 * GRUPO_H) / COLUMNAS as f32;
        let (f, c) = (i / COLUMNAS, i % COLUMNAS);
        Rectangle {
            x: MARGEN + GRUPO_H + c as f32 * w,
            y: self.y_grupo_acento() + GRUPO_V - 7.0 + f as f32 * FILA_ACENTO,
            width: w,
            height: FILA_ACENTO,
        }
    }

    // añadido en la 0.2.2
    fn rect_fondo(&self, i: usize) -> Rectangle {
        let (w, h) = celda_fondo();
        let (f, c) = (i / FONDO_COLUMNAS, i % FONDO_COLUMNAS);
        Rectangle {
            x: MARGEN + c as f32 * (w + FONDO_HUECO),
            y: self.y_fondos() + f as f32 * (h + FONDO_HUECO),
            width: w,
            height: h,
        }
    }

    // 0.1.0
    fn muestra_en(&self, x: f32, y: f32) -> Option<usize> {
        let p = Point::new(x, y);
        (0..MODOS.len()).find(|i| self.rect_modo(*i).contains(p))
    }

    // 0.1.0
    fn color_en(&self, x: f32, y: f32) -> Option<usize> {
        let p = Point::new(x, y);
        (0..Acento::TODOS.len()).find(|i| self.rect_color(*i).contains(p))
    }

    /// Una familia de fondo o, con `Some(len)`, la casilla «+».
    // 0.1.0
    fn fondo_en(&self, x: f32, y: f32) -> Option<usize> {
        let p = Point::new(x, y);
        (0..self.celdas_fondo()).find(|i| self.rect_fondo(*i).contains(p))
    }

    // --- Entrada ---------------------------------------------------------------

    /// ¿Se está moviendo algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.pulgar.animando()
            || self.hover_modo.animando()
            || self.hover_color.animando()
            || self.marca_color.animando()
            || self.hover_fondo.animando()
            || self.marca_fondo.animando()
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let refrescado = self.al_dia();
        let (m, c, f) = match punto {
            Some((x, y)) => (
                self.muestra_en(x, y),
                self.color_en(x, y),
                self.fondo_en(x, y),
            ),
            None => (None, None, None),
        };
        // Los tres `señalar` siempre, sin cortocircuito: con `||` los de
        // después no se evalúan y un realce se quedaría encendido.
        let a = self.hover_modo.señalar(m);
        let b = self.hover_color.señalar(c);
        let d = self.hover_fondo.señalar(f);
        refrescado || a || b || d
    }

    // añadido en la 0.2.2
    fn elegir_modo(&mut self, i: usize) -> Accion {
        self.modo = MODOS[i];
        self.pulgar.ir_a(i as f32);
        if tema::efectos_reducidos() {
            self.pulgar.fijar(i as f32);
        }
        // Con Claro u Oscuro se sabe ya qué variante de fondos toca; con
        // Automático lo resuelve el compositor y lo recoge `al_dia`.
        if let ModoTema::Claro | ModoTema::Oscuro = self.modo {
            let claro = self.modo == ModoTema::Claro;
            if claro != self.fondos_claro {
                let (fondos, elegido) = Self::fondos_para(claro, self.acento);
                self.vistas = vistas_de(&fondos);
                self.fondos = fondos;
                self.fondos_claro = claro;
                self.marca_fondo.señalar(elegido);
                self.marca_fondo.terminar();
            }
        }
        self.accion()
    }

    // añadido en la 0.2.2
    fn elegir_color(&mut self, i: usize) -> Accion {
        self.acento = Acento::TODOS[i];
        self.marca_color.señalar(Some(i));
        self.accion()
    }

    // añadido en la 0.2.2
    fn elegir_fondo(&mut self, i: usize) -> Accion {
        if i == self.fondos.len() {
            return Accion::Lanzar("bookos-settings --page fondos".into());
        }
        self.marca_fondo.señalar(Some(i));
        let familia = &self.fondos[i];
        // El fondo va en su propia acción y no en `Apariencia`: son dos claves
        // distintas del fichero y elegir un fondo no toca el tema.
        Accion::Fondo {
            claro: familia.claro.clone(),
            oscuro: familia.oscuro.clone(),
        }
    }

    // 0.1.0
    // modified 0.2.2 -- selector de modo y casilla «+»
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        self.al_dia();
        if let Some(i) = self.muestra_en(x, y) {
            return Some(self.elegir_modo(i));
        }
        if let Some(i) = self.fondo_en(x, y) {
            return Some(self.elegir_fondo(i));
        }
        let i = self.color_en(x, y)?;
        Some(self.elegir_color(i))
    }

    // 0.1.0
    fn accion(&self) -> Accion {
        Accion::Apariencia {
            modo: self.modo,
            acento: self.acento,
        }
    }

    /// Tab pasa de sección; las flechas eligen dentro (izquierda y derecha en
    /// fila, arriba y abajo de fila en fila); Intro abre «+».
    // 0.1.0
    // modified 0.2.2 -- teclado (HIG §11)
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        self.al_dia();
        let mover = |actual: usize, n: usize, columnas: usize, tecla: &T| -> Option<usize> {
            let paso: isize = match tecla {
                T::Izquierda => -1,
                T::Derecha => 1,
                T::Arriba => -(columnas as isize),
                T::Abajo => columnas as isize,
                _ => return None,
            };
            let nuevo = actual as isize + paso;
            (0..n as isize).contains(&nuevo).then_some(nuevo as usize)
        };
        match (tecla, self.foco) {
            (T::Escape, _) => Tecla::Cerrar,
            (T::Tabulador, foco) => {
                self.foco = Some(match foco {
                    None => Foco::Modo,
                    Some(Foco::Modo) => Foco::Acento,
                    Some(Foco::Acento) if self.fondos.is_empty() => Foco::Mas,
                    Some(Foco::Acento) => Foco::Fondo,
                    Some(Foco::Fondo) => Foco::Mas,
                    Some(Foco::Mas) => Foco::Modo,
                });
                Tecla::Consumida
            }
            (T::Intro, Some(Foco::Mas)) => Tecla::Hacer(self.elegir_fondo(self.fondos.len())),
            (tecla, Some(Foco::Modo)) => {
                let i = MODOS.iter().position(|m| *m == self.modo).unwrap_or(0);
                match mover(i, MODOS.len(), MODOS.len(), &tecla) {
                    Some(j) => Tecla::Hacer(self.elegir_modo(j)),
                    None => Tecla::Ignorada,
                }
            }
            (tecla, Some(Foco::Acento)) => {
                let i = indice(self.acento).unwrap_or(0);
                match mover(i, Acento::TODOS.len(), COLUMNAS, &tecla) {
                    Some(j) => Tecla::Hacer(self.elegir_color(j)),
                    None => Tecla::Ignorada,
                }
            }
            (tecla, Some(Foco::Fondo)) => {
                let i = self.marca_fondo.actual().unwrap_or(0);
                match mover(i, self.fondos.len(), FONDO_COLUMNAS, &tecla) {
                    Some(j) => Tecla::Hacer(self.elegir_fondo(j)),
                    None => Tecla::Ignorada,
                }
            }
            _ => Tecla::Ignorada,
        }
    }

    // --- Dibujo ----------------------------------------------------------------

    /// Los colores, todos opacos: la tarjeta, sus grupos un escalón por
    /// encima, la tinta y la gris (#6C6C70 en claro, HIG §2), el separador.
    // añadido en la 0.2.2
    fn paleta(&self) -> Paleta {
        let claro = tema::es_claro();
        let card = tema::card();
        let tx = tema::texto();
        let grupo = tema::superficie_en_tarjeta();
        Paleta {
            card,
            grupo,
            realce: tema::mezclar(grupo, tx, 0.06),
            tx,
            tx2: if claro {
                Color::from_rgb8(0x6c, 0x6c, 0x70)
            } else {
                tema::texto2()
            },
            sep: tema::mezclar(card, tx, 0.1),
        }
    }

    /// El selector de modo, el de «Modo de energía»: cápsula del grupo con un
    /// pulgar en la tinta que se desliza; la opción elegida en el color de la
    /// tarjeta y semibold, las demás en gris. La marca no es solo de color: es
    /// la píldora y el peso.
    // añadido en la 0.2.2
    fn selector(&self, p: &Paleta) -> PanelElement<'_> {
        let ancho = ancho_util() - 2.0 * SANGRIA;
        let w = (ancho - 2.0 * SELECTOR_RELLENO) / MODOS.len() as f32;
        let alto = SELECTOR - 2.0 * SELECTOR_RELLENO;
        let foco = self.foco == Some(Foco::Modo);
        let (tx, tx2, card) = (p.tx, p.tx2, p.card);
        // Desde el borde de la cápsula: el rebote de M_SEGMENTADO entra en el
        // relleno en vez de cortarse en 0.
        let pulgar = row![
            Space::new().width(Length::Fixed(
                (SELECTOR_RELLENO + w * self.pulgar.valor()).max(0.0)
            )),
            caja(
                Space::new().into(),
                w,
                alto,
                tx,
                alto / 2.0,
                foco.then_some(tema::acento())
            ),
        ]
        // A todo el ancho: la pila toma el tamaño de su primera capa, y con el
        // pulgar en la primera opción medía un tercio y recortaba las otras.
        .width(Length::Fixed(ancho))
        .padding(Padding::ZERO.top(SELECTOR_RELLENO))
        .height(Length::Fixed(SELECTOR));
        let posicion = self.pulgar.valor();
        let opciones = MODOS.iter().enumerate().fold(row![], |fila, (i, modo)| {
            // modified 0.2.3 -- la etiqueta sigue a la píldora, como en Energía
            // Cuánto la tapa la píldora: cambia de tinta y de peso según
            // pasa por encima, no de golpe al elegir.
            let on = (1.0 - (posicion - i as f32).abs()).clamp(0.0, 1.0);
            let señalada = self.hover_modo.intensidad(i);
            let nombre = match modo {
                ModoTema::Automatico => t("Automático", "Auto"),
                ModoTema::Claro => t("Claro", "Light"),
                ModoTema::Oscuro => t("Oscuro", "Dark"),
            };
            fila.push(
                container(
                    text(nombre)
                        .size(13.0)
                        .font(super::control::peso(if on > 0.5 {
                            iced_core::font::Weight::Semibold
                        } else {
                            iced_core::font::Weight::Medium
                        }))
                        .color(tema::mezclar(tema::mezclar(tx2, tx, señalada), card, on))
                        .wrapping(iced_core::text::Wrapping::None),
                )
                .center_x(Length::Fixed(w))
                .center_y(Length::Fixed(alto)),
            )
        });
        container(caja(
            stack![pulgar, container(opciones).padding(SELECTOR_RELLENO)].into(),
            ancho,
            SELECTOR,
            p.grupo,
            SELECTOR / 2.0,
            None,
        ))
        .padding(Padding {
            left: SANGRIA,
            right: SANGRIA,
            ..Padding::ZERO
        })
        .into()
    }

    /// Doce círculos de 24 en su grupo. Elegido como iOS: un aro de su propio
    /// color separado por el grupo y una ✓ en la tinta que se lee encima.
    // añadido en la 0.2.2
    fn acentos(&self, p: &Paleta) -> PanelElement<'_> {
        let w = (ancho_util() - 2.0 * GRUPO_H) / COLUMNAS as f32;
        let foco = self.foco == Some(Foco::Acento);
        let mut col = column![];
        for f in 0..filas() {
            let mut fila = row![];
            for c in 0..COLUMNAS {
                let i = f * COLUMNAS + c;
                let Some(acento) = Acento::TODOS.get(i).copied() else {
                    break;
                };
                let franjas = (acento == Acento::Automatico).then(|| self.franjas());
                let color = match acento {
                    Acento::Automatico => tema::paleta_automatica()[0],
                    a => a.color(),
                };
                let marca = self.marca_color.intensidad(i);
                let señalada = self.hover_color.intensidad(i);
                let circulo: PanelElement<'_> = match franjas {
                    Some(imagen) => iced_widget::image(imagen)
                        .width(Length::Fixed(CIRCULO))
                        .height(Length::Fixed(CIRCULO))
                        .into(),
                    // Un color casi igual que el grupo (Arena en claro) lleva un
                    // borde fino de la tinta: si no, el círculo no se ve (HIG §2,
                    // indicadores a 3:1 respecto a lo de al lado).
                    None if (luz(color) - luz(p.grupo)).abs() < 0.12 => caja(
                        caja(
                            Space::new().into(),
                            CIRCULO - 2.0,
                            CIRCULO - 2.0,
                            color,
                            CIRCULO / 2.0 - 1.0,
                            None,
                        ),
                        CIRCULO,
                        CIRCULO,
                        tema::mezclar(p.grupo, p.tx, 0.25),
                        CIRCULO / 2.0,
                        None,
                    ),
                    None => caja(
                        Space::new().into(),
                        CIRCULO,
                        CIRCULO,
                        color,
                        CIRCULO / 2.0,
                        None,
                    ),
                };
                let check = crate::icono::propio("comprobado").map(|ic| {
                    let tinta = if acento == Acento::Automatico {
                        Color::WHITE
                    } else {
                        tema::tinta_sobre(color)
                    };
                    crate::icono::ver_teñido_propio(&ic, 12.0 * marca, tinta)
                });
                let mut dentro = stack![circulo];
                if let Some(check) = check
                    && marca > 0.01
                {
                    dentro = dentro.push(
                        container(check)
                            .center_x(Length::Fixed(CIRCULO))
                            .center_y(Length::Fixed(CIRCULO)),
                    );
                }
                // El aro: el color del acento por fuera y el del grupo como aire.
                // Con el foco del teclado sobre el elegido, el aro es la tinta.
                let anillo = if foco && marca > 0.5 {
                    p.tx
                } else {
                    tema::mezclar(tema::mezclar(p.grupo, p.realce, señalada), color, marca)
                };
                let lado = CIRCULO + 2.0 * ARO;
                let anillado = caja(
                    caja(
                        dentro.into(),
                        CIRCULO + ARO,
                        CIRCULO + ARO,
                        p.grupo,
                        (CIRCULO + ARO) / 2.0,
                        None,
                    ),
                    lado,
                    lado,
                    anillo,
                    lado / 2.0,
                    None,
                );
                fila = fila.push(container(anillado).center_x(Length::Fixed(w)).height(
                    Length::Fixed(if f + 1 < filas() {
                        FILA_ACENTO
                    } else {
                        CIRCULO + ARO * 2.0
                    }),
                ));
            }
            col = col.push(fila);
        }
        caja(
            container(col)
                .padding(Padding {
                    top: GRUPO_V - ARO,
                    bottom: GRUPO_V - ARO,
                    left: GRUPO_H,
                    right: GRUPO_H,
                })
                .into(),
            ancho_util(),
            self.alto_grupo_acento(),
            p.grupo,
            tema::R_CONTROL,
            None,
        )
    }

    /// La rejilla de fondos y la casilla «+». Elegido: aro del acento con aire
    /// de la tarjeta; señalado: aro del grupo.
    // añadido en la 0.2.2
    fn fondos(&self, p: &Paleta) -> PanelElement<'_> {
        let (w, h) = celda_fondo();
        let (iw, ih) = (w - 2.0 * ARO, h - 2.0 * ARO);
        let foco = self.foco;
        let mut col = column![].spacing(FONDO_HUECO);
        let mut fila = row![].spacing(FONDO_HUECO);
        for i in 0..self.celdas_fondo() {
            let señalada = self.hover_fondo.intensidad(i);
            let celda: PanelElement<'_> = if i == self.fondos.len() {
                let mas = crate::icono::propio("vista-anadir")
                    .map(|ic| {
                        crate::icono::ver_teñido_propio(
                            &ic,
                            16.0,
                            tema::mezclar(p.tx2, p.tx, señalada),
                        )
                    })
                    .unwrap_or_else(|| text("+").size(18.0).color(p.tx2).into());
                let anillo = if foco == Some(Foco::Mas) {
                    tema::acento()
                } else {
                    p.card
                };
                caja(
                    caja(
                        mas,
                        iw,
                        ih,
                        tema::mezclar(p.grupo, p.realce, señalada),
                        FONDO_RADIO,
                        None,
                    ),
                    w,
                    h,
                    anillo,
                    FONDO_RADIO + ARO,
                    None,
                )
            } else {
                let marca = self.marca_fondo.intensidad(i);
                let imagen: PanelElement<'_> = match &self.vistas[i] {
                    Some(v) => iced_widget::image(v.clone())
                        .width(Length::Fixed(iw))
                        .height(Length::Fixed(ih))
                        .content_fit(iced_core::ContentFit::Fill)
                        .into(),
                    None => caja(Space::new().into(), iw, ih, p.grupo, FONDO_RADIO, None),
                };
                let mut anillo = tema::mezclar(
                    tema::mezclar(p.card, p.realce, señalada),
                    tema::acento(),
                    marca,
                );
                if foco == Some(Foco::Fondo) && marca > 0.5 {
                    anillo = p.tx;
                }
                // El aire entre el aro y la imagen es la tarjeta: con el acento
                // azul sobre un fondo azul, sin él no se vería el aro.
                caja(
                    caja(imagen, iw + 2.0, ih + 2.0, p.card, FONDO_RADIO + 1.0, None),
                    w,
                    h,
                    anillo,
                    FONDO_RADIO + ARO,
                    None,
                )
            };
            fila = fila.push(celda);
            if (i + 1) % FONDO_COLUMNAS == 0 || i + 1 == self.celdas_fondo() {
                col = col.push(std::mem::replace(&mut fila, row![].spacing(FONDO_HUECO)));
            }
        }
        col.into()
    }

    // 0.1.0
    // modified 0.2.2 -- la mesa del lienzo: selector, grupo de acentos, aros opacos, «+»
    pub fn view(&self) -> PanelElement<'_> {
        let p = self.paleta();
        let rotulo = |texto: &str| -> PanelElement<'static> {
            container(
                text(texto.to_owned())
                    .size(12.0)
                    .font(super::control::peso(iced_core::font::Weight::Medium))
                    .color(p.tx2),
            )
            .padding(Padding {
                left: SANGRIA,
                ..Padding::ZERO
            })
            .height(Length::Fixed(ROTULO))
            .into()
        };
        let sep = |p: &Paleta| -> PanelElement<'static> {
            let c = p.sep;
            container(
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(0.5))
                    .style(move |_: &iced_widget::Theme| container::Style {
                        background: Some(c.into()),
                        ..Default::default()
                    }),
            )
            .padding(Padding {
                left: SANGRIA,
                right: SANGRIA,
                ..Padding::ZERO
            })
            .into()
        };
        let contenido = column![
            Space::new().height(Length::Fixed(2.0)),
            container(
                text(t("Apariencia", "Appearance"))
                    .size(15.0)
                    .font(super::control::peso(iced_core::font::Weight::Semibold))
                    .color(p.tx)
            )
            .padding(Padding {
                left: SANGRIA,
                ..Padding::ZERO
            })
            .height(Length::Fixed(TITULO)),
            Space::new().height(Length::Fixed(14.0)),
            self.selector(&p),
            Space::new().height(Length::Fixed(AIRE)),
            sep(&p),
            Space::new().height(Length::Fixed(AIRE)),
            rotulo(t("Color de acento", "Accent color")),
            Space::new().height(Length::Fixed(ROTULO_BAJO)),
            self.acentos(&p),
            Space::new().height(Length::Fixed(AIRE)),
            sep(&p),
            Space::new().height(Length::Fixed(AIRE)),
            rotulo(t("Fondo de pantalla", "Wallpaper")),
            Space::new().height(Length::Fixed(ROTULO_BAJO)),
            self.fondos(&p),
        ];
        super::control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Los colores de la tarjeta; ver [`Apariencia::paleta`].
// añadido en la 0.2.2
#[derive(Clone, Copy)]
struct Paleta {
    card: Color,
    grupo: Color,
    realce: Color,
    tx: Color,
    tx2: Color,
    sep: Color,
}

/// Luminancia aproximada, de 0 a 1, para ver si dos colores se confunden.
// añadido en la 0.2.2
fn luz(c: Color) -> f32 {
    0.299 * c.r + 0.587 * c.g + 0.114 * c.b
}

/// Una caja de color opaco con su radio y, si se da, un borde de foco.
// añadido en la 0.2.2
fn caja<'a>(
    contenido: PanelElement<'a>,
    w: f32,
    h: f32,
    fondo: Color,
    radio: f32,
    foco: Option<Color>,
) -> PanelElement<'a> {
    container(contenido)
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .center_x(Length::Fixed(w))
        .center_y(Length::Fixed(h))
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: radio.into(),
                width: if foco.is_some() { 2.0 } else { 0.0 },
                color: foco.unwrap_or(Color::TRANSPARENT),
            },
            ..Default::default()
        })
        .into()
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
type EntradaCatalogo = (crate::fondos::Familia, Option<f32>);

static CATALOGO: std::sync::OnceLock<
    std::sync::Mutex<[Option<Vec<EntradaCatalogo>>; 2]>,
> = std::sync::OnceLock::new();

type ClaveVista = (std::path::PathBuf, Option<std::time::SystemTime>, u32, u32);

static CACHE_VISTAS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<ClaveVista, iced_core::image::Handle>>,
> = std::sync::OnceLock::new();

// added 0.2.3
fn leer_catalogo(es_claro: bool) -> Vec<EntradaCatalogo> {
    crate::fondos::instaladas(es_claro)
        .into_iter()
        .map(|f| {
            let t = crate::fondos::tono(&f);
            (f, t)
        })
        .collect()
}

// added 0.2.3
fn refrescar_catalogo(es_claro: bool) {
    let entradas = leer_catalogo(es_claro);
    let cache = CATALOGO.get_or_init(Default::default);
    cache.lock().unwrap_or_else(|e| e.into_inner())[es_claro as usize] = Some(entradas);
}

// added 0.2.3
fn catalogo_cacheado(es_claro: bool) -> Vec<EntradaCatalogo> {
    let cache = CATALOGO.get_or_init(Default::default);
    if let Some(guardado) =
        cache.lock().unwrap_or_else(|e| e.into_inner())[es_claro as usize].clone()
    {
        return guardado;
    }
    let entradas = leer_catalogo(es_claro);
    cache.lock().unwrap_or_else(|e| e.into_inner())[es_claro as usize] = Some(entradas.clone());
    entradas
}

/// Precalienta en segundo plano las miniaturas SVG de la variante opuesta al
/// abrir la tarjeta: cuando el usuario pulsa «Claro» u «Oscuro», `elegir_modo`
/// ya las tiene en memoria y no bloquea el primer fotograma de la píldora.
// added 0.2.3
fn precalentar_variante(es_claro: bool) {
    let _ = std::thread::Builder::new()
        .name("apariencia-vistas".into())
        .spawn(move || {
            let entradas = leer_catalogo(es_claro);
            let familias: Vec<crate::fondos::Familia> =
                entradas.iter().map(|(f, _)| f.clone()).collect();
            let _ = vistas_de(&familias);
            let cache = CATALOGO.get_or_init(Default::default);
            cache.lock().unwrap_or_else(|e| e.into_inner())[es_claro as usize] = Some(entradas);
        });
}

// added 0.2.3
fn ordenar_catalogo_por_afinidad(entradas: &mut [EntradaCatalogo], acento: Acento) {
    let Some(objetivo) = tono_de_acento(acento) else {
        return;
    };
    entradas.sort_by(|a, b| {
        let d = |t: Option<f32>| {
            t.map(|tono| crate::fondos::distancia_de_tono(tono, objetivo))
                .unwrap_or(f32::MAX)
        };
        d(a.1).total_cmp(&d(b.1)).then_with(|| a.0.nombre.cmp(&b.0.nombre))
    });
}

// 0.1.0
#[cfg(test)]
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
    let (w, h) = celda_fondo();
    let (iw, ih) = (w - 2.0 * ARO, h - 2.0 * ARO);
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
// modified 0.2.3 -- cachea el Handle ya redondeado por ruta y fecha de modificación
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
    let modificado = std::fs::metadata(ruta).ok().and_then(|m| m.modified().ok());
    let clave = (ruta.to_path_buf(), modificado, pw, ph);
    let cache = CACHE_VISTAS.get_or_init(Default::default);
    if let Some(guardada) = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&clave)
        .cloned()
    {
        return Some(guardada);
    }
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
        FONDO_RADIO * RESOLUCION,
        crate::Alfa::Recto,
    );
    let handle = iced_core::image::Handle::from_rgba(pw, ph, rgba);
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(clave, handle.clone());
    Some(handle)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Los doce colores responden cada uno en su sitio, sus zonas no se
    /// pisan y la rejilla cabe en la tarjeta.
    #[test]
    fn los_colores_responden_en_su_sitio() {
        let a = Apariencia::new(ModoTema::Oscuro);
        for i in 0..Acento::TODOS.len() {
            let r = a.rect_color(i);
            let c = r.center();
            assert_eq!(a.color_en(c.x, c.y), Some(i), "el color {i}");
            assert!(
                r.width >= 28.0 && r.height >= 28.0,
                "zona de {i} menor que 28 (HIG §11)"
            );
            for j in i + 1..Acento::TODOS.len() {
                assert!(
                    r.intersection(&a.rect_color(j))
                        .is_none_or(|x| x.width * x.height < 0.5),
                    "{i} pisa {j}"
                );
            }
        }
        assert!(a.y_grupo_acento() + a.alto_grupo_acento() <= a.size().1);
    }

    #[test]
    fn elegir_un_color_lo_pide_y_lo_marca() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        let c = a.rect_color(2).center();
        assert_eq!(
            a.pulsar(c.x, c.y),
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

    /// Cada fondo pide su pareja entera y la casilla «+» abre Ajustes.
    #[test]
    fn la_rejilla_de_fondos_y_el_mas() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        for i in 0..a.fondos.len() {
            let c = a.rect_fondo(i).center();
            let esperada = a.fondos[i].clone();
            assert_eq!(
                a.pulsar(c.x, c.y),
                Some(Accion::Fondo {
                    claro: esperada.claro,
                    oscuro: esperada.oscuro,
                })
            );
            assert_eq!(a.marca_fondo.actual(), Some(i));
        }
        let mas = a.rect_fondo(a.fondos.len()).center();
        assert_eq!(
            a.pulsar(mas.x, mas.y),
            Some(Accion::Lanzar("bookos-settings --page fondos".into()))
        );
        let ultima = a.rect_fondo(a.fondos.len());
        assert!(
            ultima.y + ultima.height <= a.size().1 - MARGEN + 0.01,
            "la rejilla se sale"
        );
    }

    /// El selector elige cada modo en su tercio y el pulgar va hacia él.
    #[test]
    fn el_selector_elige_el_modo() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        for (i, esperado) in MODOS.iter().enumerate() {
            let c = a.rect_modo(i).center();
            assert_eq!(
                a.pulsar(c.x, c.y),
                Some(Accion::Apariencia {
                    modo: *esperado,
                    acento: tema::acento_actual(),
                })
            );
        }
        assert_eq!(a.modo, ModoTema::Oscuro);
    }

    /// Tab recorre las secciones y las flechas eligen dentro (HIG §11).
    #[test]
    fn el_teclado_recorre_y_elige() {
        use crate::TeclaPulsada as T;
        let mut a = Apariencia::new(ModoTema::Claro);
        assert!(matches!(a.tecla(T::Tabulador), Tecla::Consumida));
        assert!(matches!(
            a.tecla(T::Derecha),
            Tecla::Hacer(Accion::Apariencia {
                modo: ModoTema::Oscuro,
                ..
            })
        ));
        assert!(
            matches!(a.tecla(T::Derecha), Tecla::Ignorada),
            "no da la vuelta"
        );
        a.tecla(T::Tabulador);
        let antes = indice(a.acento).unwrap_or(0);
        if antes + COLUMNAS < Acento::TODOS.len() {
            a.tecla(T::Abajo);
            assert_eq!(indice(a.acento), Some(antes + COLUMNAS));
        }
        a.tecla(T::Tabulador);
        if !a.fondos.is_empty() {
            a.tecla(T::Tabulador);
        }
        assert_eq!(a.foco, Some(Foco::Mas));
        assert!(matches!(a.tecla(T::Intro), Tecla::Hacer(Accion::Lanzar(_))));
        assert!(matches!(a.tecla(T::Escape), Tecla::Cerrar));
    }

    /// `BOOKOS_APARIENCIA_PNG=/tmp/a.png` vuelca la tarjeta en oscuro y claro.
    #[test]
    fn volcar_png() {
        let Ok(ruta) = std::env::var("BOOKOS_APARIENCIA_PNG") else {
            return;
        };
        for (sufijo, tema_, modo) in [
            ("oscuro", tema::Tema::Oscuro, ModoTema::Oscuro),
            ("claro", tema::Tema::Claro, ModoTema::Automatico),
        ] {
            tema::aplicar(tema_);
            let config = crate::Config {
                tema: tema_,
                ..Default::default()
            };
            let mut shell = crate::Shell::con_config(1646, 1.0, config);
            shell.abrir(super::super::Emergente::Apariencia(Apariencia::new(modo)));
            let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_emergente(&mut buf);
            for p in buf.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            image::save_buffer(
                ruta.replace(".png", &format!("-{sufijo}.png")),
                &buf,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .expect("se escribe el PNG");
        }
    }
}
