//! El selector conserva el foco del cliente: las teclas de búsqueda se quedan
//! en el shell y solo la elección final pide insertar texto al compositor.

mod catalogo;

use std::sync::LazyLock;

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, image as imagen, pin, row, svg, text};

use super::{Ancla, Tecla, control, t};
use crate::icono;
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};
use catalogo::CATALOGO;

// Swash no dibuja la fuente COLRv1 de esta sesión. El atlas de Noto evita
// que una fuente instalada decida si las 354 celdas están visibles o vacías.
static GLIFOS: LazyLock<Vec<imagen::Handle>> = LazyLock::new(|| {
    let atlas = image::load_from_memory(include_bytes!("../../assets/emojis/noto.png"))
        .expect("el atlas integrado es un PNG válido");
    CATALOGO
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let celda = atlas.crop_imm((i as u32 % 18) * 64, (i as u32 / 18) * 64, 64, 64);
            imagen::Handle::from_rgba(64, 64, celda.into_rgba8().into_raw())
        })
        .collect()
});

const MARGEN: f32 = 12.0;
const COLUMNAS: usize = 8;
const FILAS: usize = 6;
const CELDA: f32 = 38.0;
const ANCHO: f32 = MARGEN * 2.0 + COLUMNAS as f32 * CELDA;
const ALTO: f32 = 444.0;
const REJILLA_Y: f32 = 124.0;
const CATEGORIAS_Y: f32 = 392.0;
const CATEGORIA_ANCHO: f32 = (ANCHO - MARGEN * 2.0) / 9.0;

struct Emoji {
    simbolo: &'static str,
    es: &'static str,
    en: &'static str,
    categoria: usize,
}

const CATEGORIAS: [(&str, &str, &[u8]); 9] = [
    (
        "Recientes",
        "Recent",
        include_bytes!("../../assets/iconos/emoji-clock.svg"),
    ),
    (
        "Caras y personas",
        "Smileys & people",
        include_bytes!("../../assets/iconos/emoji-smile.svg"),
    ),
    (
        "Animales y naturaleza",
        "Animals & nature",
        include_bytes!("../../assets/iconos/emoji-leaf.svg"),
    ),
    (
        "Comida y bebida",
        "Food & drink",
        include_bytes!("../../assets/iconos/emoji-coffee.svg"),
    ),
    (
        "Actividades",
        "Activities",
        include_bytes!("../../assets/iconos/emoji-trophy.svg"),
    ),
    (
        "Viajes y lugares",
        "Travel & places",
        include_bytes!("../../assets/iconos/emoji-plane.svg"),
    ),
    (
        "Objetos",
        "Objects",
        include_bytes!("../../assets/iconos/emoji-lightbulb.svg"),
    ),
    (
        "Símbolos",
        "Symbols",
        include_bytes!("../../assets/iconos/emoji-heart.svg"),
    ),
    (
        "Banderas",
        "Flags",
        include_bytes!("../../assets/iconos/emoji-flag.svg"),
    ),
];

pub struct Emojis {
    ancla: Ancla,
    consulta: String,
    categoria: usize,
    recientes: Vec<usize>,
    resultados: Vec<usize>,
    elegido: usize,
    /// Cuánto se ha bajado por la rejilla, en píxeles lógicos. En píxeles y no
    /// en filas para que el touchpad mueva la rejilla con el dedo: por filas
    /// enteras saltaba 38 px de golpe y no se veía hacia dónde iba.
    desplazamiento: tema::Transicion,
    señalado: tema::Realce,
    categoria_señalada: tema::Realce,
    /// Dónde se agarró la barra del título, en coordenadas de la tarjeta,
    /// mientras se arrastra para moverla. Es una ventana de herramientas y
    /// estorba donde salga: hay que poder llevarla a un lado.
    agarre: Option<(f32, f32)>,
}

/// Lo que dura cada tramo del desplazamiento. Es el de un popover: lo bastante
/// corto para que el touchpad, que manda un evento cada pocos milisegundos, no
/// vaya por detrás del dedo, y lo bastante largo para suavizar un clic de
/// rueda o una tecla.
const D_DESPLAZAR: std::time::Duration = tema::D_POPOVER;

impl Emojis {
    pub fn new(x: f32, y: f32, recientes: &[String]) -> Self {
        let recientes: Vec<_> = recientes
            .iter()
            .filter_map(|s| CATALOGO.iter().position(|e| e.simbolo == s))
            .collect();
        let mut selector = Self {
            ancla: Ancla::Punto { x, y },
            consulta: String::new(),
            categoria: usize::from(recientes.is_empty()),
            recientes,
            resultados: Vec::new(),
            elegido: 0,
            desplazamiento: tema::Transicion::nueva(0.0, D_DESPLAZAR, tema::C_SUAVE),
            señalado: tema::Realce::nuevo(),
            categoria_señalada: tema::Realce::nuevo(),
            agarre: None,
        };
        selector.filtrar();
        selector
    }

    pub fn size(&self) -> (f32, f32) {
        (ANCHO, ALTO)
    }
    pub fn ancla(&self) -> Ancla {
        self.ancla
    }
    pub fn animando(&self) -> bool {
        self.señalado.animando()
            || self.categoria_señalada.animando()
            || self.desplazamiento.animando()
    }

    /// Hasta dónde se puede bajar: lo justo para que la última fila quede
    /// apoyada abajo.
    fn maximo(&self) -> f32 {
        self.resultados
            .len()
            .div_ceil(COLUMNAS)
            .saturating_sub(FILAS) as f32
            * CELDA
    }

    /// Lleva la rejilla a `y`, recortado a lo que hay. `true` si se mueve.
    fn desplazar_a(&mut self, y: f32) -> bool {
        let y = y.clamp(0.0, self.maximo());
        if tema::efectos_reducidos() {
            let cambia = self.desplazamiento.destino() != y;
            self.desplazamiento.fijar(y);
            return cambia;
        }
        self.desplazamiento.ir_a(y)
    }

    fn filtrar(&mut self) {
        let consulta = normalizar(self.consulta.trim());
        self.resultados = if !consulta.is_empty() {
            CATALOGO
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let claves = normalizar(&format!("{} {} {}", e.simbolo, e.es, e.en));
                    consulta
                        .split_whitespace()
                        .all(|palabra| claves.contains(palabra))
                        .then_some(i)
                })
                .collect()
        } else if self.categoria == 0 {
            self.recientes.clone()
        } else {
            CATALOGO
                .iter()
                .enumerate()
                .filter_map(|(i, e)| (e.categoria == self.categoria).then_some(i))
                .collect()
        };
        self.elegido = 0;
        // Otra lista empieza arriba y sin viaje: animar desde donde estaba la
        // anterior sería mover contenido que ya no existe.
        self.desplazamiento.fijar(0.0);
        self.señalado.señalar(None);
    }

    fn cambiar_categoria(&mut self, categoria: usize) {
        self.categoria = categoria;
        self.consulta.clear();
        self.filtrar();
    }

    fn elegir(&self, indice: usize) -> Option<Accion> {
        let e = CATALOGO.get(*self.resultados.get(indice)?)?;
        Some(Accion::InsertarEmoji(e.simbolo.to_string()))
    }

    fn celda_en(&self, x: f32, y: f32) -> Option<usize> {
        if !(MARGEN..ANCHO - MARGEN).contains(&x)
            || !(REJILLA_Y..REJILLA_Y + FILAS as f32 * CELDA).contains(&y)
        {
            return None;
        }
        let col = ((x - MARGEN) / CELDA) as usize;
        let fila = ((y - REJILLA_Y + self.desplazamiento.valor()) / CELDA) as usize;
        let i = fila * COLUMNAS + col;
        (i < self.resultados.len()).then_some(i)
    }

    fn categoria_en(x: f32, y: f32) -> Option<usize> {
        ((MARGEN..ANCHO - MARGEN).contains(&x) && (CATEGORIAS_Y..CATEGORIAS_Y + 40.0).contains(&y))
            .then(|| ((x - MARGEN) / CATEGORIA_ANCHO) as usize)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        // Arrastrando, el punto llega relativo a donde está la tarjeta ahora:
        // lo que se ha separado del agarre es lo que hay que moverla.
        if let (Some((ax, ay)), Some((px, py)), Ancla::Punto { x, y }) =
            (self.agarre, punto, &mut self.ancla)
        {
            *x += px - ax;
            *y += py - ay;
            return px != ax || py != ay;
        }
        let celda = punto.and_then(|(x, y)| self.celda_en(x, y));
        let categoria = punto.and_then(|(x, y)| Self::categoria_en(x, y));
        let celda_cambio = self.señalado.señalar(celda);
        let categoria_cambio = self.categoria_señalada.señalar(categoria);
        celda_cambio || categoria_cambio
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(categoria) = Self::categoria_en(x, y) {
            self.cambiar_categoria(categoria);
        } else if (ANCHO - MARGEN - 24.0..ANCHO - MARGEN).contains(&x)
            && (MARGEN..MARGEN + 24.0).contains(&y)
        {
            return Some(Accion::CerrarEmergente);
        } else if let Some(indice) = self.celda_en(x, y) {
            return self.elegir(indice);
        } else if (MARGEN..MARGEN + 24.0).contains(&y) {
            self.agarre = Some((x, y));
        }
        None
    }

    pub fn soltar(&mut self) -> bool {
        self.agarre.take().is_some()
    }

    pub fn agarrado(&self) -> bool {
        self.agarre.is_some()
    }

    fn asegurar_visible(&mut self) {
        // Contra el destino, no contra lo que se ve: con varias flechas
        // seguidas la rejilla aún va de camino y cada tecla tiene que sumar.
        let arriba = (self.elegido / COLUMNAS) as f32 * CELDA;
        let actual = self.desplazamiento.destino();
        if arriba < actual {
            self.desplazar_a(arriba);
        } else if arriba + CELDA > actual + FILAS as f32 * CELDA {
            self.desplazar_a(arriba + CELDA - FILAS as f32 * CELDA);
        }
        self.señalado.señalar(None);
    }

    pub fn desplazar(&mut self, dy: f32) -> bool {
        let destino = self.desplazamiento.destino() + dy;
        if !self.desplazar_a(destino) {
            return false;
        }
        // La elegida con el teclado no puede quedarse fuera de la vista: Intro
        // insertaría un emoji que no estás viendo.
        let y = self.desplazamiento.destino();
        let primera = (y / CELDA).ceil() as usize;
        let ultima =
            ((y / CELDA).floor() as usize + FILAS).min(self.resultados.len().div_ceil(COLUMNAS));
        self.elegido = self.elegido.clamp(
            primera * COLUMNAS,
            (ultima * COLUMNAS)
                .min(self.resultados.len())
                .max(primera * COLUMNAS + 1)
                - 1,
        );
        self.señalado.señalar(None);
        true
    }

    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        use TeclaPulsada as T;
        match tecla {
            T::Escape => return Tecla::Cerrar,
            T::Intro => {
                return self
                    .elegir(self.elegido)
                    .map_or(Tecla::Consumida, Tecla::Hacer);
            }
            T::Caracter(c) if !c.is_control() && self.consulta.len() < 256 => {
                self.consulta.push(c);
                self.filtrar();
            }
            T::Retroceso => {
                self.consulta.pop();
                self.filtrar();
            }
            T::BorrarTodo => {
                self.consulta.clear();
                self.filtrar();
            }
            T::Tabulador => self.cambiar_categoria((self.categoria + 1) % CATEGORIAS.len()),
            T::Izquierda => self.elegido = self.elegido.saturating_sub(1),
            T::Derecha => {
                self.elegido = (self.elegido + 1).min(self.resultados.len().saturating_sub(1))
            }
            T::Arriba => self.elegido = self.elegido.saturating_sub(COLUMNAS),
            T::Abajo => {
                self.elegido =
                    (self.elegido + COLUMNAS).min(self.resultados.len().saturating_sub(1))
            }
            T::Inicio => self.elegido = 0,
            T::Fin => self.elegido = self.resultados.len().saturating_sub(1),
            T::PaginaArriba => self.elegido = self.elegido.saturating_sub(COLUMNAS * FILAS),
            T::PaginaAbajo => {
                self.elegido =
                    (self.elegido + COLUMNAS * FILAS).min(self.resultados.len().saturating_sub(1))
            }
            _ => {}
        }
        self.asegurar_visible();
        Tecla::Consumida
    }

    pub fn view(&self) -> PanelElement<'_> {
        let ancho = ANCHO - 2.0 * MARGEN;
        let titulo = row![
            text(t("Emojis", "Emoji"))
                .size(14)
                .font(control::peso(iced_core::font::Weight::Semibold))
                .color(tema::texto()),
            Space::new().width(Length::Fill),
            container(icono::ver_teñido_propio(
                &icono::propio("cerrar").expect("cerrar es un icono integrado"),
                14.0,
                tema::TEXTO2
            ))
            .center(24.0),
        ]
        .align_y(Vertical::Center)
        .height(24.0);
        let consulta = if self.consulta.is_empty() {
            t("Buscar emojis", "Search emoji").to_string()
        } else {
            super::recortar_texto(&self.consulta, ancho - 56.0, 14.0)
        };
        let campo = container(
            row![
                icono::ver_teñido_propio(
                    &icono::propio("buscar").expect("buscar es un icono integrado"),
                    16.0,
                    tema::acento()
                ),
                text(consulta).size(14).color(if self.consulta.is_empty() {
                    tema::TEXTO2
                } else {
                    tema::texto()
                }),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        )
        .padding([0, 12])
        .width(ancho)
        .height(40.0)
        .align_y(Vertical::Center)
        .style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                width: 1.0,
                color: tema::alfa(tema::acento(), 0.45),
            },
            ..Default::default()
        });
        let (es, en, _) = CATEGORIAS[self.categoria];
        let etiqueta = if self.consulta.trim().is_empty() {
            t(es, en)
        } else {
            t("Resultados", "Results")
        };
        let cabecera = row![
            text(etiqueta).size(12).color(tema::TEXTO2),
            Space::new().width(Length::Fill),
            text(self.resultados.len().to_string())
                .size(11)
                .color(tema::TEXTO2)
        ]
        .height(20.0)
        .align_y(Vertical::Center);
        // Una fila de más: a mitad de camino asoman dos medias, la de arriba y
        // la de abajo. El recorte del contenedor quita lo que sobra.
        let y = self.desplazamiento.valor();
        let primera = (y / CELDA).floor() as usize;
        let mut rejilla = column![];
        for fila in primera..primera + FILAS + 1 {
            let mut celdas = row![];
            for col in 0..COLUMNAS {
                let i = fila * COLUMNAS + col;
                let contenido: PanelElement<'_> = if let Some(&indice) = self.resultados.get(i) {
                    let hover = self.señalado.intensidad(i);
                    let elegido = i == self.elegido;
                    container(imagen(GLIFOS[indice].clone()).width(36).height(36))
                        .center(CELDA)
                        .style(move |_| container::Style {
                            background: Some(
                                tema::alfa(
                                    tema::acento(),
                                    if elegido { 0.10 } else { 0.07 * hover },
                                )
                                .into(),
                            ),
                            border: Border {
                                radius: tema::R_ITEM_POPOVER.into(),
                                width: if elegido { 1.0 } else { 0.0 },
                                color: tema::alfa(tema::acento(), 0.45),
                            },
                            ..Default::default()
                        })
                        .into()
                } else {
                    Space::new().width(CELDA).height(CELDA).into()
                };
                celdas = celdas.push(contenido);
            }
            rejilla = rejilla.push(celdas);
        }
        let rejilla: PanelElement<'_> = if self.resultados.is_empty() {
            container(
                column![
                    text(if self.categoria == 0 && self.consulta.is_empty() {
                        t("Aún no hay recientes", "No recent emoji")
                    } else {
                        t("Sin resultados", "No results")
                    })
                    .size(14)
                    .color(tema::texto()),
                    text(t(
                        "Prueba otra categoría o búsqueda",
                        "Try another category or search"
                    ))
                    .size(11)
                    .color(tema::TEXTO2),
                ]
                .spacing(8)
                .align_x(Horizontal::Center),
            )
            .width(ancho)
            .height(CELDA * FILAS as f32)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
        } else {
            container(pin(rejilla).y(primera as f32 * CELDA - y))
                .width(ancho)
                .height(CELDA * FILAS as f32)
                .clip(true)
                .into()
        };
        let detalle = self.señalado.actual().unwrap_or(self.elegido);
        let descripcion = self
            .resultados
            .get(detalle)
            .map(|&i| {
                let e = &CATALOGO[i];
                if *super::ES {
                    e.es
                } else {
                    e.en.split(' ').next().unwrap_or(e.en)
                }
            })
            .unwrap_or(t("Tab cambia de categoría", "Tab switches category"));
        let pie = row![
            text(super::recortar_texto(descripcion, ancho - 80.0, 11.0))
                .size(11)
                .color(tema::TEXTO2),
            Space::new().width(Length::Fill),
            text(t("Intro  ·  Esc", "Enter  ·  Esc"))
                .size(11)
                .color(tema::TEXTO2)
        ]
        .height(24.0)
        .align_y(Vertical::Center);
        let mut categorias = row![];
        for (i, &(_, _, bytes)) in CATEGORIAS.iter().enumerate() {
            let activo = self.consulta.trim().is_empty() && self.categoria == i;
            let hover = self.categoria_señalada.intensidad(i);
            let color = if activo { tema::acento() } else { tema::TEXTO2 };
            categorias = categorias.push(
                container(
                    svg(svg::Handle::from_memory(bytes))
                        .width(18)
                        .height(18)
                        .style(move |_, _| svg::Style { color: Some(color) }),
                )
                .width(CATEGORIA_ANCHO)
                .height(40)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: Some(
                        tema::alfa(tema::acento(), if activo { 0.10 } else { 0.07 * hover }).into(),
                    ),
                    border: Border {
                        radius: tema::R_BOTON_PEQUENO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        control::tarjeta(
            column![
                titulo,
                Space::new().height(8),
                campo,
                Space::new().height(12),
                cabecera,
                Space::new().height(8),
                rejilla,
                Space::new().height(8),
                pie,
                Space::new().height(8),
                categorias
            ]
            .into(),
            ANCHO,
            MARGEN,
        )
    }
}

fn normalizar(texto: &str) -> String {
    texto
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buscar(selector: &mut Emojis, consulta: &str) {
        selector.tecla(TeclaPulsada::BorrarTodo);
        for c in consulta.chars() {
            selector.tecla(TeclaPulsada::Caracter(c));
        }
    }

    #[test]
    fn buscar_ignora_tildes_y_combina_palabras_en_ambos_idiomas() {
        let mut selector = Emojis::new(0.0, 0.0, &[]);
        buscar(&mut selector, "CORAZON AZUL");
        assert!(
            matches!(selector.tecla(TeclaPulsada::Intro), Tecla::Hacer(Accion::InsertarEmoji(s)) if s == "💙")
        );
        buscar(&mut selector, "blue heart");
        assert_eq!(selector.resultados.len(), 1);
        buscar(&mut selector, "🧑‍💻");
        assert!(
            matches!(selector.tecla(TeclaPulsada::Intro), Tecla::Hacer(Accion::InsertarEmoji(s)) if s == "🧑‍💻")
        );
    }

    #[test]
    fn sin_resultados_no_se_inserta_nada_y_escape_cierra() {
        let mut selector = Emojis::new(0.0, 0.0, &[]);
        buscar(&mut selector, "zzzzzz");
        selector.tecla(TeclaPulsada::Abajo);
        assert!(!selector.desplazar(200.0));
        assert!(matches!(
            selector.tecla(TeclaPulsada::Intro),
            Tecla::Consumida
        ));
        assert!(matches!(
            selector.tecla(TeclaPulsada::Escape),
            Tecla::Cerrar
        ));
    }

    #[test]
    fn el_teclado_desplaza_la_rejilla_y_el_raton_elige_la_celda_visible() {
        let mut selector = Emojis::new(0.0, 0.0, &[]);
        selector.tecla(TeclaPulsada::Fin);
        let y = selector.desplazamiento.destino();
        assert!(y > 0.0);
        // Ya llegado: el clic cae donde se ve la rejilla al final del viaje.
        selector.desplazamiento.fijar(y);
        let primera = (y / CELDA) as usize;
        let esperado = selector.elegir(primera * COLUMNAS);
        assert_eq!(selector.pulsar(MARGEN + 1.0, REJILLA_Y + 1.0), esperado);
        selector.tecla(TeclaPulsada::Inicio);
        assert_eq!(selector.desplazamiento.destino(), 0.0);
    }

    /// La rueda y el touchpad mueven la rejilla por píxeles, no por filas, y
    /// con animación; los extremos la paran.
    #[test]
    fn la_rueda_desplaza_suave_y_se_para_en_los_extremos() {
        let mut selector = Emojis::new(0.0, 0.0, &[]);
        assert!(selector.desplazar(10.0));
        assert_eq!(selector.desplazamiento.destino(), 10.0);
        assert!(selector.animando(), "el tramo se anima");
        assert!(!selector.desplazar(-50.0) || selector.desplazamiento.destino() == 0.0);
        assert!(!selector.desplazar(-5.0), "arriba del todo no hay más");
        selector.desplazar(1.0e6);
        assert_eq!(selector.desplazamiento.destino(), selector.maximo());
    }

    /// Arrastrar el título mueve la tarjeta; soltar la deja donde está.
    #[test]
    fn el_titulo_arrastra_la_tarjeta() {
        let mut selector = Emojis::new(100.0, 100.0, &[]);
        assert_eq!(selector.pulsar(40.0, MARGEN + 10.0), None);
        assert!(selector.agarrado());
        assert!(selector.puntero(Some((70.0, MARGEN + 30.0))));
        assert!(selector.soltar());
        assert_eq!(selector.ancla(), Ancla::Punto { x: 130.0, y: 120.0 });
    }

    #[test]
    fn recientes_conserva_las_secuencias_y_cambiar_categoria_borra_la_busqueda() {
        let mut selector = Emojis::new(0.0, 0.0, &["🧑‍💻".into(), "🇪🇸".into()]);
        assert_eq!(selector.categoria, 0);
        assert_eq!(selector.resultados.len(), 2);
        assert_eq!(
            selector.pulsar(MARGEN + CELDA + 1.0, REJILLA_Y + 1.0),
            Some(Accion::InsertarEmoji("🇪🇸".into()))
        );
        buscar(&mut selector, "corazon");
        selector.pulsar(MARGEN + CATEGORIA_ANCHO * 3.5, CATEGORIAS_Y + 20.0);
        assert_eq!(selector.categoria, 3);
        assert!(selector.consulta.is_empty());
        assert!(
            selector
                .resultados
                .iter()
                .all(|&i| CATALOGO[i].categoria == 3)
        );
    }
}
