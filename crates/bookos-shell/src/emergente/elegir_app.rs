//! Selector «Abrir con…» del portal (`org.freedesktop.impl.portal.AppChooser`).
//!
//! Muestra las aplicaciones propuestas por `xdg-desktop-portal` con sus iconos
//! y nombres reales, permite filtrar escribiendo sin abrir otro cuadro de
//! diálogo y mantiene una altura fija de cinco filas visibles para evitar que
//! el búfer se rehaga mientras el usuario busca o desplaza la lista.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length, Padding};
use iced_widget::{Space, column, container, row, stack, text};

use super::color_carpeta::{SOMBRA, sombra_dialogo};
use super::{Ancla, FilaAccesible, Tecla, t};
use crate::apps;
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const ANCHO: f32 = 380.0;
const LATERAL: f32 = 24.0;
const INTERIOR: f32 = ANCHO - LATERAL * 2.0;
const ARRIBA: f32 = 24.0;
const ABAJO: f32 = 20.0;
const T_TITULO: f32 = 17.0;
const LH_TITULO: f32 = 22.0;
const T_SUB: f32 = 12.5;
const LH_SUB: f32 = 18.0;
const BUSCAR_H: f32 = 38.0;
const FILA_H: f32 = 48.0;
const VISIBLES: usize = 5;
const LISTA_H: f32 = FILA_H * VISIBLES as f32;
const AIRE_BOTONES: f32 = 14.0;
const BOTON: f32 = 44.0;
const HUECO_BOTON: f32 = 10.0;
const DOBLE_CLIC: Duration = Duration::from_millis(400);

/// Datos de apertura del diálogo `org.freedesktop.impl.portal.AppChooser`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeticionElegirApp {
    pub id: u32,
    pub subtitulo: String,
    pub opciones: Vec<String>,
    pub ultima: Option<String>,
}

#[derive(Clone)]
struct Entrada {
    desktop_id: String,
    nombre: String,
    detalle: Option<String>,
    icono_nombre: String,
    icono: Option<Icono>,
    normalizado: String,
    recomendada: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pieza {
    Fila(usize),
    Cancelar,
    Abrir,
}

pub struct ElegirApp {
    id: u32,
    subtitulo: String,
    recomendadas: Vec<Entrada>,
    catalogo: Vec<Entrada>,
    consulta: String,
    elegida: Option<String>,
    primera: usize,
    scroll_px: f32,
    alto: f32,
    sombra: [Icono; 4],
    señalado: Option<Pieza>,
    ultimo_clic: Option<(String, Instant)>,
}

fn nombre_desde_id(id: &str) -> String {
    let base = id.strip_suffix(".desktop").unwrap_or(id);
    let cola = base.rsplit('.').next().unwrap_or(base);
    let mut chars = cola.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => id.to_string(),
    }
}

fn construir_entrada(desktop_id: String, recomendada: bool) -> Entrada {
    if let Some(app) = apps::por_desktop_id(&desktop_id) {
        let icono = icono::cargar(&app.icono);
        Entrada {
            desktop_id,
            nombre: app.nombre,
            detalle: app.detalle,
            icono_nombre: app.icono,
            icono,
            normalizado: app.normalizado,
            recomendada,
        }
    } else {
        let nombre = nombre_desde_id(&desktop_id);
        let normalizado = apps::normalizar(&nombre);
        Entrada {
            desktop_id,
            nombre,
            detalle: None,
            icono_nombre: String::new(),
            icono: None,
            normalizado,
            recomendada,
        }
    }
}

fn construir_recomendadas(opciones: &[String], ultima: Option<&str>) -> Vec<Entrada> {
    let mut vistas = HashSet::new();
    let mut lista = Vec::with_capacity(opciones.len());
    for op in opciones {
        let limpia = op.strip_suffix(".desktop").unwrap_or(op);
        if !vistas.insert(limpia.to_string()) {
            continue;
        }
        lista.push(construir_entrada(limpia.to_string(), true));
    }
    if let Some(u) = ultima {
        let base = u.strip_suffix(".desktop").unwrap_or(u);
        if let Some(pos) = lista.iter().position(|e| e.desktop_id == base)
            && pos > 0
        {
            let elem = lista.remove(pos);
            lista.insert(0, elem);
        }
    }
    lista
}

impl ElegirApp {
    pub fn new(peticion: PeticionElegirApp) -> Self {
        let recomendadas =
            construir_recomendadas(&peticion.opciones, peticion.ultima.as_deref());
        let catalogo = apps::catalogo_con_id()
            .into_iter()
            .map(|(desktop_id, app)| {
                let icono = icono::cargar(&app.icono);
                Entrada {
                    desktop_id,
                    nombre: app.nombre,
                    detalle: app.detalle,
                    icono_nombre: app.icono,
                    icono,
                    normalizado: app.normalizado,
                    recomendada: false,
                }
            })
            .collect();
        Self::con_entradas(
            peticion.id,
            peticion.subtitulo,
            recomendadas,
            catalogo,
            peticion.ultima,
        )
    }

    fn con_entradas(
        id: u32,
        subtitulo: String,
        recomendadas: Vec<Entrada>,
        catalogo: Vec<Entrada>,
        ultima: Option<String>,
    ) -> Self {
        let ultima_base = ultima
            .as_deref()
            .map(|u| u.strip_suffix(".desktop").unwrap_or(u));
        let elegida = ultima_base
            .and_then(|u| {
                recomendadas
                    .iter()
                    .chain(catalogo.iter())
                    .find(|e| e.desktop_id == u)
                    .map(|e| e.desktop_id.clone())
            })
            .or_else(|| recomendadas.first().map(|e| e.desktop_id.clone()))
            .or_else(|| catalogo.first().map(|e| e.desktop_id.clone()));

        let mut d = Self {
            id,
            subtitulo,
            recomendadas,
            catalogo,
            consulta: String::new(),
            elegida,
            primera: 0,
            scroll_px: 0.0,
            alto: 0.0,
            sombra: sombra_dialogo(ANCHO, 200.0),
            señalado: None,
            ultimo_clic: None,
        };
        d.alto = d.y_botones() + BOTON + ABAJO;
        d.sombra = sombra_dialogo(ANCHO, d.alto);
        d
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn cancelar(&self) -> Accion {
        Accion::PortalElegirApp {
            id: self.id,
            eleccion: None,
        }
    }

    fn aceptar(&self) -> Option<Accion> {
        let elegida = self.elegida_valida()?;
        Some(Accion::PortalElegirApp {
            id: self.id,
            eleccion: Some(elegida.desktop_id.clone()),
        })
    }

    pub fn size(&self) -> (f32, f32) {
        (ANCHO + SOMBRA * 2.0, self.alto + SOMBRA * 2.0)
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    pub fn animando(&self) -> bool {
        false
    }

    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    pub fn recargar_iconos(&mut self) {
        for e in self
            .recomendadas
            .iter_mut()
            .chain(self.catalogo.iter_mut())
        {
            if !e.icono_nombre.is_empty() {
                e.icono = icono::cargar(&e.icono_nombre);
            }
        }
    }

    pub fn actualizar_opciones(&mut self, opciones: Vec<String>) -> bool {
        self.recomendadas = construir_recomendadas(&opciones, None);
        self.ajustar_seleccion_y_scroll();
        true
    }

    fn filtradas(&self) -> Vec<&Entrada> {
        let q = self.consulta.trim();
        if q.is_empty() {
            return if !self.recomendadas.is_empty() {
                self.recomendadas.iter().collect()
            } else {
                self.catalogo.iter().collect()
            };
        }
        let norm = apps::normalizar(q);
        let lower = q.to_lowercase();
        let casa = |e: &&Entrada| {
            e.normalizado.contains(&norm) || e.desktop_id.to_lowercase().contains(&lower)
        };
        let mut vistas = HashSet::new();
        let mut salida = Vec::new();
        for e in self.recomendadas.iter().filter(casa) {
            if vistas.insert(e.desktop_id.as_str()) {
                salida.push(e);
            }
        }
        for e in self.catalogo.iter().filter(casa) {
            if vistas.insert(e.desktop_id.as_str()) {
                salida.push(e);
            }
        }
        salida
    }

    fn elegida_valida(&self) -> Option<&Entrada> {
        let lista = self.filtradas();
        if let Some(id) = &self.elegida
            && let Some(e) = lista.iter().find(|e| &e.desktop_id == id)
        {
            return Some(e);
        }
        lista.first().copied()
    }

    fn indice_elegido(&self, lista: &[&Entrada]) -> Option<usize> {
        let id = self.elegida.as_ref()?;
        lista.iter().position(|e| &e.desktop_id == id)
    }

    fn ajustar_seleccion_y_scroll(&mut self) {
        let lista = self.filtradas();
        let total = lista.len();
        if total == 0 {
            self.elegida = None;
            self.primera = 0;
            self.scroll_px = 0.0;
            return;
        }
        let pos = self.indice_elegido(&lista).unwrap_or(0);
        self.elegida = Some(lista[pos].desktop_id.clone());
        let max_primera = total.saturating_sub(VISIBLES);
        if pos < self.primera {
            self.primera = pos;
        } else if pos >= self.primera + VISIBLES {
            self.primera = pos + 1 - VISIBLES;
        }
        self.primera = self.primera.min(max_primera);
        self.scroll_px = (self.primera as f32) * FILA_H;
    }

    fn mover_seleccion(&mut self, delta: isize) {
        let lista = self.filtradas();
        let total = lista.len();
        if total == 0 {
            return;
        }
        let actual = self.indice_elegido(&lista).unwrap_or(0) as isize;
        let nuevo = (actual + delta).clamp(0, total as isize - 1) as usize;
        self.elegida = Some(lista[nuevo].desktop_id.clone());
        self.ajustar_seleccion_y_scroll();
    }

    fn y_lista(&self) -> f32 {
        ARRIBA + LH_TITULO + 4.0 + LH_SUB + 12.0 + BUSCAR_H + 10.0
    }

    fn y_botones(&self) -> f32 {
        self.y_lista() + LISTA_H + AIRE_BOTONES
    }

    fn pieza_en(&self, x: f32, y: f32) -> Option<Pieza> {
        let (x, y) = (x - SOMBRA - LATERAL, y - SOMBRA);
        if !(0.0..=INTERIOR).contains(&x) {
            return None;
        }
        let y_lista = self.y_lista();
        if (y_lista..y_lista + LISTA_H).contains(&y) {
            let rel = ((y - y_lista) / FILA_H).floor() as usize;
            let total = self.filtradas().len();
            if self.primera + rel < total {
                return Some(Pieza::Fila(rel));
            }
            return None;
        }
        let y_bot = self.y_botones();
        if (y_bot..y_bot + BOTON).contains(&y) {
            let medio = (INTERIOR - HUECO_BOTON) / 2.0;
            if x <= medio {
                return Some(Pieza::Cancelar);
            }
            if x >= medio + HUECO_BOTON {
                return Some(Pieza::Abrir);
            }
        }
        None
    }

    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let mut out = Vec::new();
        let y_buscar = SOMBRA + ARRIBA + LH_TITULO + 4.0 + LH_SUB + 12.0;
        out.push(FilaAccesible {
            id: "elegir-app:buscar".into(),
            rol: accesskit::Role::Search,
            nombre: t("Buscar aplicación", "Search application").into(),
            valor: self.consulta.clone(),
            rect: (SOMBRA + LATERAL, y_buscar, INTERIOR, BUSCAR_H),
            marcada: None,
            seleccionada: false,
        });
        let lista = self.filtradas();
        let y_lista = SOMBRA + self.y_lista();
        for (vis, e) in lista.iter().skip(self.primera).take(VISIBLES).enumerate() {
            let marcada = self.elegida.as_deref() == Some(e.desktop_id.as_str());
            out.push(FilaAccesible {
                id: format!("elegir-app:fila:{}", e.desktop_id),
                rol: accesskit::Role::ListBoxOption,
                nombre: e.nombre.clone(),
                valor: e.detalle.clone().unwrap_or_default(),
                rect: (
                    SOMBRA + LATERAL,
                    y_lista + vis as f32 * FILA_H,
                    INTERIOR,
                    FILA_H,
                ),
                marcada: Some(marcada),
                seleccionada: marcada || self.señalado == Some(Pieza::Fila(vis)),
            });
        }
        let y_bot = SOMBRA + self.y_botones();
        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        out.push(FilaAccesible {
            id: "elegir-app:cancelar".into(),
            rol: accesskit::Role::Button,
            nombre: t("Cancelar", "Cancel").into(),
            valor: String::new(),
            rect: (SOMBRA + LATERAL, y_bot, medio, BOTON),
            marcada: None,
            seleccionada: self.señalado == Some(Pieza::Cancelar),
        });
        out.push(FilaAccesible {
            id: "elegir-app:abrir".into(),
            rol: accesskit::Role::Button,
            nombre: t("Abrir", "Open").into(),
            valor: String::new(),
            rect: (
                SOMBRA + LATERAL + medio + HUECO_BOTON,
                y_bot,
                medio,
                BOTON,
            ),
            marcada: None,
            seleccionada: self.señalado == Some(Pieza::Abrir),
        });
        out
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.pieza_en(x, y));
        let cambia = nueva != self.señalado;
        self.señalado = nueva;
        cambia
    }

    pub fn desplazar(&mut self, dy: f32) -> bool {
        if !dy.is_finite() {
            return false;
        }
        let total = self.filtradas().len();
        let max_primera = total.saturating_sub(VISIBLES);
        let max_px = max_primera as f32 * FILA_H;
        self.scroll_px = (self.scroll_px + dy).clamp(0.0, max_px);
        let nueva = (self.scroll_px / FILA_H).round() as usize;
        let nueva = nueva.min(max_primera);
        if nueva == self.primera {
            return false;
        }
        self.primera = nueva;
        true
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let pieza = self.pieza_en(x, y)?;
        match pieza {
            Pieza::Fila(vis) => {
                let idx = self.primera + vis;
                let id = self.filtradas().get(idx).map(|e| e.desktop_id.clone())?;
                let ahora = Instant::now();
                let doble = self
                    .ultimo_clic
                    .as_ref()
                    .is_some_and(|(prev, t)| prev == &id && ahora.saturating_duration_since(*t) <= DOBLE_CLIC);
                self.elegida = Some(id.clone());
                self.ultimo_clic = Some((id, ahora));
                if doble { self.aceptar() } else { None }
            }
            Pieza::Cancelar => Some(self.cancelar()),
            Pieza::Abrir => self.aceptar(),
        }
    }

    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        match tecla {
            TeclaPulsada::Escape if !self.consulta.is_empty() => {
                self.consulta.clear();
                self.primera = 0;
                self.scroll_px = 0.0;
                self.ajustar_seleccion_y_scroll();
                Tecla::Consumida
            }
            TeclaPulsada::Escape => Tecla::Hacer(self.cancelar()),
            TeclaPulsada::Intro => match self.aceptar() {
                Some(a) => Tecla::Hacer(a),
                None => Tecla::Consumida,
            },
            TeclaPulsada::Arriba => {
                self.mover_seleccion(-1);
                Tecla::Consumida
            }
            TeclaPulsada::Abajo => {
                self.mover_seleccion(1);
                Tecla::Consumida
            }
            TeclaPulsada::PaginaArriba => {
                self.mover_seleccion(-(VISIBLES as isize));
                Tecla::Consumida
            }
            TeclaPulsada::PaginaAbajo => {
                self.mover_seleccion(VISIBLES as isize);
                Tecla::Consumida
            }
            TeclaPulsada::Caracter(c) if !c.is_control() => {
                self.consulta.push(c);
                self.primera = 0;
                self.scroll_px = 0.0;
                self.elegida = self.filtradas().first().map(|e| e.desktop_id.clone());
                Tecla::Consumida
            }
            TeclaPulsada::Pegar(texto) => {
                let antes = self.consulta.len();
                for c in texto.chars() {
                    if !c.is_control() {
                        self.consulta.push(c);
                    }
                }
                if self.consulta.len() != antes {
                    self.primera = 0;
                    self.scroll_px = 0.0;
                    self.elegida = self.filtradas().first().map(|e| e.desktop_id.clone());
                }
                Tecla::Consumida
            }
            TeclaPulsada::Retroceso => {
                if self.consulta.pop().is_some() {
                    self.primera = 0;
                    self.scroll_px = 0.0;
                    self.elegida = self.filtradas().first().map(|e| e.desktop_id.clone());
                }
                Tecla::Consumida
            }
            TeclaPulsada::BorrarTodo => {
                if !self.consulta.is_empty() {
                    self.consulta.clear();
                    self.primera = 0;
                    self.scroll_px = 0.0;
                    self.ajustar_seleccion_y_scroll();
                }
                Tecla::Consumida
            }
            _ => Tecla::Consumida,
        }
    }

    fn boton(&self, etiqueta: &'static str, pieza: Pieza, ancho: f32) -> PanelElement<'_> {
        let principal = pieza == Pieza::Abrir;
        let deshabilitado = principal && self.elegida_valida().is_none();
        let fondo = if principal {
            if deshabilitado {
                tema::alfa(tema::acento(), 0.45)
            } else if self.señalado == Some(pieza) {
                tema::mezclar(tema::acento(), tema::tinta(), 0.12)
            } else {
                tema::acento()
            }
        } else {
            let base = tema::superficie_en_tarjeta();
            if self.señalado == Some(pieza) {
                tema::mezclar(base, tema::tinta(), 0.12)
            } else {
                base
            }
        };
        let tinta = if principal {
            let t = tema::tinta_sobre(tema::acento());
            if deshabilitado {
                tema::alfa(t, 0.72)
            } else {
                t
            }
        } else {
            tema::texto()
        };
        container(
            text(etiqueta)
                .size(15)
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tinta),
        )
        .center(Length::Fill)
        .width(ancho)
        .height(BOTON)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn vista_buscar(&self) -> PanelElement<'_> {
        let contenido: PanelElement<'_> = if self.consulta.is_empty() {
            row![
                container(Space::new().width(1.5).height(16.0)).style(|_| container::Style {
                    background: Some(tema::acento().into()),
                    ..Default::default()
                }),
                Space::new().width(4.0),
                text(t("Buscar aplicación…", "Search app…"))
                    .size(13.5)
                    .color(tema::texto2()),
            ]
            .align_y(Vertical::Center)
            .into()
        } else {
            row![
                text(super::recortar_texto(&self.consulta, INTERIOR - 32.0, 13.5))
                    .size(13.5)
                    .color(tema::texto()),
                container(Space::new().width(1.5).height(16.0)).style(|_| container::Style {
                    background: Some(tema::acento().into()),
                    ..Default::default()
                }),
            ]
            .align_y(Vertical::Center)
            .into()
        };

        container(contenido)
            .padding([0, 12])
            .width(INTERIOR)
            .height(BUSCAR_H)
            .center_y(BUSCAR_H)
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    width: 1.5,
                    color: tema::acento(),
                },
                ..Default::default()
            })
            .into()
    }

    fn vista_lista(&self) -> PanelElement<'_> {
        let lista = self.filtradas();
        let total = lista.len();
        if total == 0 {
            return container(
                text(t("Sin coincidencias", "No matching apps"))
                    .size(13)
                    .color(tema::texto2()),
            )
            .width(INTERIOR)
            .height(LISTA_H)
            .center(LISTA_H)
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into();
        }

        let elegida_id = self.elegida_valida().map(|e| e.desktop_id.as_str());
        let buscando = !self.consulta.trim().is_empty();
        let mut col = column![];
        for (vis, e) in lista.iter().skip(self.primera).take(VISIBLES).enumerate() {
            let seleccionada = elegida_id == Some(e.desktop_id.as_str());
            let señalada = self.señalado == Some(Pieza::Fila(vis));
            let icono_el: PanelElement<'_> = match &e.icono {
                Some(ic) => icono::ver(ic, 30.0, 30.0),
                None => {
                    let inicial = e
                        .nombre
                        .chars()
                        .next()
                        .map_or_else(|| "?".into(), |c| c.to_uppercase().collect::<String>());
                    container(
                        text(inicial)
                            .size(13)
                            .font(Font {
                                weight: Weight::Bold,
                                ..Font::DEFAULT
                            })
                            .color(tema::texto()),
                    )
                    .width(30.0)
                    .height(30.0)
                    .center(30.0)
                    .style(|_| container::Style {
                        background: Some(tema::card().into()),
                        border: Border {
                            radius: 8.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into()
                }
            };
            let sublinea = e
                .detalle
                .as_deref()
                .or_else(|| (buscando && e.recomendada).then_some(t("Recomendada", "Recommended")));
            let ancho_texto = INTERIOR - 24.0 - 30.0 - 10.0 - 20.0;
            let mut textos = column![
                text(super::recortar_texto(&e.nombre, ancho_texto, 13.5))
                    .size(13.5)
                    .font(Font {
                        weight: Weight::Medium,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto()),
            ];
            if let Some(sub) = sublinea {
                textos = textos.push(
                    text(super::recortar_texto(sub, ancho_texto, 11.0))
                        .size(11)
                        .color(tema::texto2()),
                );
            }
            let marca: PanelElement<'_> = if seleccionada {
                text("✓")
                    .size(14)
                    .font(Font {
                        weight: Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tema::acento())
                    .into()
            } else {
                Space::new().width(14.0).into()
            };
            let fila = container(
                row![
                    icono_el,
                    Space::new().width(10.0),
                    textos.width(Length::Fill),
                    marca,
                ]
                .align_y(Vertical::Center),
            )
            .padding(Padding {
                top: 0.0,
                right: 12.0,
                bottom: 0.0,
                left: 12.0,
            })
            .width(INTERIOR)
            .height(FILA_H)
            .center_y(FILA_H)
            .style(move |_| container::Style {
                background: if seleccionada {
                    Some(tema::alfa(tema::acento(), 0.16).into())
                } else if señalada {
                    Some(tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.06).into())
                } else {
                    None
                },
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    width: if seleccionada { 1.0 } else { 0.0 },
                    color: if seleccionada {
                        tema::alfa(tema::acento(), 0.45)
                    } else {
                        Color::TRANSPARENT
                    },
                },
                ..Default::default()
            });
            col = col.push(fila);
        }

        let contenido: PanelElement<'_> = if total > VISIBLES {
            let ratio_h = (VISIBLES as f32 / total as f32).clamp(0.15, 1.0);
            let barra_h = (LISTA_H - 12.0) * ratio_h;
            let max_primera = (total - VISIBLES).max(1) as f32;
            let desplazamiento_barra =
                ((LISTA_H - 12.0 - barra_h) * (self.primera as f32 / max_primera)).max(0.0);
            let indicador = container(
                column![
                    Space::new().height(6.0 + desplazamiento_barra),
                    container(Space::new().width(3.0).height(barra_h)).style(|_| {
                        container::Style {
                            background: Some(tema::alfa(tema::texto2(), 0.45).into()),
                            border: Border {
                                radius: 1.5.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }),
                ]
                .width(5.0),
            )
            .width(INTERIOR)
            .height(LISTA_H)
            .align_x(Horizontal::Right)
            .padding(Padding {
                right: 3.0,
                ..Default::default()
            });
            stack![col, indicador].into()
        } else {
            col.into()
        };

        container(contenido)
            .width(INTERIOR)
            .height(LISTA_H)
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        let col = column![
            Space::new().height(ARRIBA),
            container(
                text(t("Abrir con", "Open with"))
                    .size(T_TITULO)
                    .font(Font {
                        weight: Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto()),
            )
            .height(LH_TITULO)
            .center_y(LH_TITULO),
            Space::new().height(4.0),
            container(
                text(super::recortar_texto(&self.subtitulo, INTERIOR, T_SUB))
                    .size(T_SUB)
                    .color(tema::texto2()),
            )
            .height(LH_SUB)
            .center_y(LH_SUB),
            Space::new().height(12.0),
            self.vista_buscar(),
            Space::new().height(10.0),
            self.vista_lista(),
            Space::new().height(AIRE_BOTONES),
            row![
                self.boton(t("Cancelar", "Cancel"), Pieza::Cancelar, medio),
                self.boton(t("Abrir", "Open"), Pieza::Abrir, medio),
            ]
            .spacing(HUECO_BOTON),
        ]
        .align_x(Horizontal::Center);

        let tarjeta = container(container(col).padding([0, LATERAL as u16]))
            .width(ANCHO)
            .height(self.alto)
            .style(|_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            });
        let (ancho, alto) = self.size();
        let franja = SOMBRA + tema::R_DIALOGO;
        let lado = alto - franja * 2.0;
        let [arriba, izquierda, derecha, abajo] = &self.sombra;
        stack![
            column![
                icono::ver(arriba, ancho, franja),
                row![
                    icono::ver(izquierda, SOMBRA, lado),
                    Space::new().width(ANCHO),
                    icono::ver(derecha, SOMBRA, lado),
                ],
                icono::ver(abajo, ancho, franja),
            ],
            container(tarjeta).padding(SOMBRA)
        ]
        .into()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn entrada(id: &str, nombre: &str, recomendada: bool) -> Entrada {
        Entrada {
            desktop_id: id.into(),
            nombre: nombre.into(),
            detalle: None,
            icono_nombre: String::new(),
            icono: None,
            normalizado: apps::normalizar(nombre),
            recomendada,
        }
    }

    fn selector_prueba() -> ElegirApp {
        let recs = vec![
            entrada("org.mozilla.firefox", "Firefox", true),
            entrada("com.example.Alpha", "Alpha", true),
            entrada("com.example.Beta", "Beta", true),
            entrada("com.example.Gamma", "Gamma", true),
            entrada("com.example.Delta", "Delta", true),
            entrada("com.example.Epsilon", "Epsilon", true),
        ];
        let cat = vec![
            entrada("org.mozilla.firefox", "Firefox", false),
            entrada("org.gimp.GIMP", "GIMP", false),
        ];
        ElegirApp::con_entradas(
            12,
            "informe.pdf".into(),
            recs,
            cat,
            Some("com.example.Beta".into()),
        )
    }

    #[test]
    fn ultima_eleccion_queda_preseleccionada() {
        let e = selector_prueba();
        assert_eq!(e.elegida.as_deref(), Some("com.example.Beta"));
    }

    #[test]
    fn buscar_filtra_recomendadas_y_catalogo_y_escape_limpia_antes_de_cancelar() {
        let mut e = selector_prueba();
        let tam = e.size();
        for c in "gimp".chars() {
            assert!(matches!(e.tecla(TeclaPulsada::Caracter(c)), Tecla::Consumida));
        }
        assert_eq!(e.size(), tam);
        assert_eq!(e.elegida.as_deref(), Some("org.gimp.GIMP"));
        assert!(matches!(
            e.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::PortalElegirApp {
                id: 12,
                eleccion: Some(ref s)
            }) if s == "org.gimp.GIMP"
        ));

        assert!(matches!(e.tecla(TeclaPulsada::Escape), Tecla::Consumida));
        assert!(e.consulta.is_empty());
        assert!(matches!(
            e.tecla(TeclaPulsada::Escape),
            Tecla::Hacer(Accion::PortalElegirApp {
                id: 12,
                eleccion: None
            })
        ));
    }

    #[test]
    fn flechas_desplazan_ventana_visible_y_raton_abre() {
        let mut e = selector_prueba();
        for _ in 0..6 {
            e.tecla(TeclaPulsada::Abajo);
        }
        assert_eq!(e.elegida.as_deref(), Some("com.example.Epsilon"));
        assert_eq!(e.primera, 1);

        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        let cx_ok = SOMBRA + LATERAL + medio + HUECO_BOTON + medio / 2.0;
        let cy_bot = SOMBRA + e.y_botones() + BOTON / 2.0;
        assert!(matches!(
            e.pulsar(cx_ok, cy_bot),
            Some(Accion::PortalElegirApp {
                id: 12,
                eleccion: Some(ref s)
            }) if s == "com.example.Epsilon"
        ));
    }

    #[test]
    fn actualizar_opciones_conserva_tamano_y_seleccion_valida() {
        let mut e = selector_prueba();
        let tam = e.size();
        e.actualizar_opciones(vec!["org.mozilla.firefox".into()]);
        assert_eq!(e.size(), tam);
        assert_eq!(e.elegida.as_deref(), Some("org.mozilla.firefox"));
    }

    #[test]
    fn desplazar_no_finito_y_pegar_solo_control_no_alteran_estado() {
        let mut e = selector_prueba();
        for _ in 0..6 {
            e.tecla(TeclaPulsada::Abajo);
        }
        let elegido = e.elegida.clone();
        let primera = e.primera;
        assert!(!e.desplazar(f32::NAN));
        assert!(e.scroll_px.is_finite());
        e.tecla(TeclaPulsada::Pegar("\n\x07\t".into()));
        assert_eq!(e.elegida, elegido);
        assert_eq!(e.primera, primera);
    }
}
