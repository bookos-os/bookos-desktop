//! Historial del portapapeles (`Meta+,`): textos e imágenes recientes y fijadas.
//!
//! El selector conserva el foco del cliente: buscar, filtrar y navegar se
//! quedan en el shell, y al elegir una entrada el compositor repone la
//! selección y envía el pegado al destino que tenía el foco al abrirse.

use std::collections::HashMap;
use std::sync::Arc;

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, ContentFit, Length, Padding};
use iced_widget::{Space, column, container, image as imagen, row, stack, text};

use super::{Ancla, FilaAccesible, Tecla, control, t};
use crate::icono;
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const ANCHO: f32 = 360.0;
const ALTO: f32 = 480.0;
const MARGEN: f32 = 12.0;
const INTERIOR: f32 = ANCHO - MARGEN * 2.0;

const CAB_Y: f32 = MARGEN;
const CAB_H: f32 = 28.0;
const BUSCAR_Y: f32 = CAB_Y + CAB_H + 8.0;
const BUSCAR_H: f32 = 38.0;
const SEG_Y: f32 = BUSCAR_Y + BUSCAR_H + 8.0;
const SEG_H: f32 = 34.0;
const SUB_Y: f32 = SEG_Y + SEG_H + 8.0;
const SUB_H: f32 = 20.0;
const LISTA_Y: f32 = SUB_Y + SUB_H + 6.0;
const LISTA_H: f32 = 280.0;
const PIE_H: f32 = 20.0;

const CARD_GAP: f32 = 8.0;
const ALTO_TEXTO: f32 = 78.0;
const ALTO_IMAGEN: f32 = 138.0;
const MINI_CAJA_H: f32 = 92.0;
const BTN_LADO: f32 = 28.0;
const SEG_PAD: f32 = 3.0;
const SEG_TAB_ANCHO: f32 = (INTERIOR - SEG_PAD * 2.0) / 3.0;

/// Tope físico de la miniatura (2× sobre la caja lógica de 320×92).
const MINI_MAX_W: u32 = 640;
const MINI_MAX_H: u32 = 184;

/// Miniatura RGBA8 ya escalada y con esquinas redondeadas para tiny-skia.
#[derive(Debug, Clone)]
pub struct MiniaturaPortapapeles {
    pub ancho_original: u32,
    pub alto_original: u32,
    pub ancho: u32,
    pub alto: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Tipo de contenido de una entrada del historial.
#[derive(Debug, Clone)]
pub enum ContenidoPortapapeles {
    Texto(String),
    Imagen {
        formato: String,
        miniatura: MiniaturaPortapapeles,
    },
}

/// Snapshot serializable entre compositor y shell de un elemento del historial.
#[derive(Debug, Clone)]
pub struct ElementoPortapapeles {
    pub id: u64,
    pub fijada: bool,
    pub hora: String,
    pub origen: Option<String>,
    pub contenido: ContenidoPortapapeles,
}

/// Decodifica una imagen en memoria, la escala conservando su relación de aspecto
/// y redondea sus esquinas en los píxeles (tiny-skia ignora `Image::border_radius`).
pub fn miniatura_de_imagen(bytes: &[u8]) -> Option<MiniaturaPortapapeles> {
    let img = image::load_from_memory(bytes).ok()?;
    let ancho_original = img.width();
    let alto_original = img.height();
    if ancho_original == 0 || alto_original == 0 {
        return None;
    }
    let escalada = if ancho_original > MINI_MAX_W || alto_original > MINI_MAX_H {
        img.resize(MINI_MAX_W, MINI_MAX_H, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut rgba_img = escalada.into_rgba8();
    let ancho = rgba_img.width();
    let alto = rgba_img.height();
    if ancho == 0 || alto == 0 {
        return None;
    }
    let escala_visual = (ancho as f32 / (INTERIOR - 16.0)).max(alto as f32 / MINI_CAJA_H).max(1.0);
    let radio = (tema::R_BOTON_PEQUENO * escala_visual).min(ancho.min(alto) as f32 / 2.0);
    let mut rgba = rgba_img.as_mut();
    crate::redondear_esquinas(&mut rgba, ancho, alto, radio, crate::Alfa::Recto);
    Some(MiniaturaPortapapeles {
        ancho_original,
        alto_original,
        ancho,
        alto,
        rgba: Arc::new(rgba_img.into_raw()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filtro {
    Todo,
    Texto,
    Fotos,
}

impl Filtro {
    const TODOS: [Self; 3] = [Self::Todo, Self::Texto, Self::Fotos];

    fn indice(self) -> usize {
        match self {
            Self::Todo => 0,
            Self::Texto => 1,
            Self::Fotos => 2,
        }
    }

    fn desde_indice(i: usize) -> Self {
        Self::TODOS.get(i).copied().unwrap_or(Self::Todo)
    }

    fn etiqueta(self) -> &'static str {
        match self {
            Self::Todo => t("Todo", "All"),
            Self::Texto => t("Texto", "Text"),
            Self::Fotos => t("Fotos", "Images"),
        }
    }

    fn siguiente(self) -> Self {
        Self::desde_indice((self.indice() + 1) % Self::TODOS.len())
    }
}

/// Qué botón concreto está bajo el puntero o siendo evaluado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zona {
    Cerrar,
    Vaciar,
    Filtro(usize),
    Tarjeta(usize),
    Fijar(usize),
    Eliminar(usize),
}

pub struct Portapapeles {
    ancla: Ancla,
    elementos: Vec<ElementoPortapapeles>,
    handles: HashMap<u64, imagen::Handle>,
    visibles: Vec<usize>,
    consulta: String,
    filtro: Filtro,
    elegido: usize,
    desplazamiento: tema::Transicion,
    tarjeta_senalada: tema::Realce,
    tab_senalada: tema::Realce,
    fijar_senalado: tema::Realce,
    eliminar_senalado: tema::Realce,
    sobre_cerrar: bool,
    sobre_vaciar: bool,
    agarre: Option<(f32, f32)>,
}

impl Portapapeles {
    pub fn new(x: f32, y: f32, elementos: Vec<ElementoPortapapeles>) -> Self {
        let mut s = Self {
            ancla: Ancla::Punto { x, y },
            elementos: Vec::new(),
            handles: HashMap::new(),
            visibles: Vec::new(),
            consulta: String::new(),
            filtro: Filtro::Todo,
            elegido: 0,
            desplazamiento: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            tarjeta_senalada: tema::Realce::nuevo(),
            tab_senalada: tema::Realce::nuevo(),
            fijar_senalado: tema::Realce::nuevo(),
            eliminar_senalado: tema::Realce::nuevo(),
            sobre_cerrar: false,
            sobre_vaciar: false,
            agarre: None,
        };
        s.actualizar(elementos);
        s
    }

    /// Sustituye el historial mostrado conservando búsqueda, filtro y posición válida.
    pub fn actualizar(&mut self, elementos: Vec<ElementoPortapapeles>) {
        let id_elegido = self
            .visibles
            .get(self.elegido)
            .and_then(|&idx| self.elementos.get(idx))
            .map(|e| e.id);

        let mut nuevos_handles = HashMap::new();
        for el in &elementos {
            if let ContenidoPortapapeles::Imagen { miniatura, .. } = &el.contenido {
                let handle = self.handles.remove(&el.id).unwrap_or_else(|| {
                    imagen::Handle::from_rgba(
                        miniatura.ancho,
                        miniatura.alto,
                        (*miniatura.rgba).clone(),
                    )
                });
                nuevos_handles.insert(el.id, handle);
            }
        }
        self.handles = nuevos_handles;
        self.elementos = elementos;
        self.reordenar_fijadas_y_filtrar(false);
        if let Some(id) = id_elegido
            && let Some(pos) = self
                .visibles
                .iter()
                .position(|&idx| self.elementos[idx].id == id)
        {
            self.elegido = pos;
        } else {
            self.elegido = self.elegido.min(self.visibles.len().saturating_sub(1));
        }
        let max = self.maximo();
        if self.desplazamiento.destino() > max {
            self.desplazamiento.fijar(max);
        }
    }

    pub fn size(&self) -> (f32, f32) {
        (ANCHO, ALTO)
    }

    pub fn ancla(&self) -> Ancla {
        self.ancla
    }

    pub fn animando(&self) -> bool {
        self.tarjeta_senalada.animando()
            || self.tab_senalada.animando()
            || self.fijar_senalado.animando()
            || self.eliminar_senalado.animando()
            || self.desplazamiento.animando()
    }

    pub fn recortar_ancla(&mut self, x: f32, y: f32) {
        self.ancla = Ancla::Punto { x, y };
    }

    pub fn agarrado(&self) -> bool {
        self.agarre.is_some()
    }

    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        let limite_x = self.limite_arrastre_x();
        (MARGEN..limite_x).contains(&x) && (CAB_Y..CAB_Y + CAB_H).contains(&y)
    }

    fn limite_arrastre_x(&self) -> f32 {
        let cerrar_x0 = ANCHO - MARGEN - BTN_LADO;
        if self.elementos.is_empty() {
            cerrar_x0 - 4.0
        } else {
            cerrar_x0 - 6.0 - self.ancho_vaciar() - 4.0
        }
    }

    fn ancho_vaciar(&self) -> f32 {
        let etiqueta = t("Vaciar", "Clear");
        crate::widget::ancho_de(etiqueta, 12.0) + 20.0
    }

    fn reordenar_fijadas_y_filtrar(&mut self, reiniciar_scroll: bool) {
        let consulta = normalizar(self.consulta.trim());
        let palabras: Vec<&str> = consulta.split_whitespace().collect();

        let coincide = |el: &ElementoPortapapeles| -> bool {
            let pasa_filtro = match (self.filtro, &el.contenido) {
                (Filtro::Todo, _) => true,
                (Filtro::Texto, ContenidoPortapapeles::Texto(_)) => true,
                (Filtro::Fotos, ContenidoPortapapeles::Imagen { .. }) => true,
                _ => false,
            };
            if !pasa_filtro {
                return false;
            }
            if palabras.is_empty() {
                return true;
            }
            let base = match &el.contenido {
                ContenidoPortapapeles::Texto(tx) => {
                    format!(
                        "{} {}",
                        tx,
                        el.origen.as_deref().unwrap_or_default()
                    )
                }
                ContenidoPortapapeles::Imagen { formato, miniatura } => format!(
                    "imagen foto captura image photo picture screenshot {} {}x{} {}",
                    formato,
                    miniatura.ancho_original,
                    miniatura.alto_original,
                    el.origen.as_deref().unwrap_or_default()
                ),
            };
            let norm = normalizar(&base);
            palabras.iter().all(|p| norm.contains(p))
        };

        let mut fijadas = Vec::new();
        let mut recientes = Vec::new();
        for (i, el) in self.elementos.iter().enumerate() {
            if coincide(el) {
                if el.fijada {
                    fijadas.push(i);
                } else {
                    recientes.push(i);
                }
            }
        }
        fijadas.extend(recientes);
        self.visibles = fijadas;
        if reiniciar_scroll {
            self.elegido = 0;
            self.desplazamiento.fijar(0.0);
        } else {
            self.elegido = self.elegido.min(self.visibles.len().saturating_sub(1));
        }
        self.tarjeta_senalada.señalar(None);
        self.fijar_senalado.señalar(None);
        self.eliminar_senalado.señalar(None);
    }

    fn alto_tarjeta(el: &ElementoPortapapeles) -> f32 {
        match &el.contenido {
            ContenidoPortapapeles::Texto(_) => ALTO_TEXTO,
            ContenidoPortapapeles::Imagen { .. } => ALTO_IMAGEN,
        }
    }

    /// Devuelve `(indice_en_visibles, y_en_lista, alto_tarjeta)` para cada visible.
    fn colocar(&self) -> Vec<(usize, f32, f32)> {
        let mut out = Vec::with_capacity(self.visibles.len());
        let mut y = 0.0;
        for (vis_i, &el_i) in self.visibles.iter().enumerate() {
            let h = Self::alto_tarjeta(&self.elementos[el_i]);
            out.push((vis_i, y, h));
            y += h + CARD_GAP;
        }
        out
    }

    fn alto_total(&self) -> f32 {
        if self.visibles.is_empty() {
            return 0.0;
        }
        let ultimo = self.colocar().pop().expect("no vacío");
        ultimo.1 + ultimo.2
    }

    fn maximo(&self) -> f32 {
        (self.alto_total() - LISTA_H).max(0.0)
    }

    fn desplazar_a(&mut self, y: f32) -> bool {
        let y = y.clamp(0.0, self.maximo());
        if tema::efectos_reducidos() {
            let cambia = (self.desplazamiento.destino() - y).abs() > 0.01;
            self.desplazamiento.fijar(y);
            return cambia;
        }
        self.desplazamiento.ir_a(y)
    }

    fn asegurar_visible(&mut self) {
        let colocadas = self.colocar();
        let Some(&(_, y, h)) = colocadas.get(self.elegido) else {
            return;
        };
        let actual = self.desplazamiento.destino();
        if y < actual {
            self.desplazar_a(y);
        } else if y + h > actual + LISTA_H {
            self.desplazar_a(y + h - LISTA_H);
        }
        self.tarjeta_senalada.señalar(None);
    }

    fn zona_en(&self, x: f32, y: f32) -> Option<Zona> {
        let cerrar_x0 = ANCHO - MARGEN - BTN_LADO;
        if (cerrar_x0..ANCHO - MARGEN).contains(&x) && (CAB_Y..CAB_Y + CAB_H).contains(&y) {
            return Some(Zona::Cerrar);
        }
        if !self.elementos.is_empty() {
            let vaciar_x1 = cerrar_x0 - 6.0;
            let vaciar_x0 = vaciar_x1 - self.ancho_vaciar();
            if (vaciar_x0..vaciar_x1).contains(&x) && (CAB_Y..CAB_Y + CAB_H).contains(&y) {
                return Some(Zona::Vaciar);
            }
        }
        if (MARGEN + SEG_PAD..ANCHO - MARGEN - SEG_PAD).contains(&x)
            && (SEG_Y + SEG_PAD..SEG_Y + SEG_H - SEG_PAD).contains(&y)
        {
            let tab = ((x - MARGEN - SEG_PAD) / SEG_TAB_ANCHO) as usize;
            if tab < Filtro::TODOS.len() {
                return Some(Zona::Filtro(tab));
            }
        }
        if (MARGEN..ANCHO - MARGEN).contains(&x) && (LISTA_Y..LISTA_Y + LISTA_H).contains(&y) {
            let lx = x - MARGEN;
            let ly = y - LISTA_Y + self.desplazamiento.valor();
            for (vis_i, cy, ch) in self.colocar() {
                if (cy..cy + ch).contains(&ly) {
                    let rx = lx;
                    let ry = ly - cy;
                    let es_imagen = matches!(
                        self.elementos[self.visibles[vis_i]].contenido,
                        ContenidoPortapapeles::Imagen { .. }
                    );
                    let (btn_y0, btn_y1) = if es_imagen {
                        (8.0 + MINI_CAJA_H + 4.0, 8.0 + MINI_CAJA_H + 4.0 + BTN_LADO)
                    } else {
                        (6.0, 6.0 + BTN_LADO)
                    };
                    let btn_del_x0 = INTERIOR - 6.0 - BTN_LADO;
                    let btn_pin_x0 = btn_del_x0 - 2.0 - BTN_LADO;
                    if (btn_y0..btn_y1).contains(&ry) {
                        if (btn_del_x0..btn_del_x0 + BTN_LADO).contains(&rx) {
                            return Some(Zona::Eliminar(vis_i));
                        }
                        if (btn_pin_x0..btn_pin_x0 + BTN_LADO).contains(&rx) {
                            return Some(Zona::Fijar(vis_i));
                        }
                    }
                    return Some(Zona::Tarjeta(vis_i));
                }
            }
        }
        None
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if let (Some((ax, ay)), Some((px, py)), Ancla::Punto { x, y }) =
            (self.agarre, punto, &mut self.ancla)
        {
            *x += px - ax;
            *y += py - ay;
            return px != ax || py != ay;
        }
        let zona = punto.and_then(|(x, y)| self.zona_en(x, y));
        let (tarjeta, fijar, eliminar, tab, cerrar, vaciar) = match zona {
            Some(Zona::Tarjeta(i)) => (Some(i), None, None, None, false, false),
            Some(Zona::Fijar(i)) => (Some(i), Some(i), None, None, false, false),
            Some(Zona::Eliminar(i)) => (Some(i), None, Some(i), None, false, false),
            Some(Zona::Filtro(t)) => (None, None, None, Some(t), false, false),
            Some(Zona::Cerrar) => (None, None, None, None, true, false),
            Some(Zona::Vaciar) => (None, None, None, None, false, true),
            None => (None, None, None, None, false, false),
        };
        let mut cambio = false;
        cambio |= self.tarjeta_senalada.señalar(tarjeta);
        cambio |= self.fijar_senalado.señalar(fijar);
        cambio |= self.eliminar_senalado.señalar(eliminar);
        cambio |= self.tab_senalada.señalar(tab);
        if self.sobre_cerrar != cerrar {
            self.sobre_cerrar = cerrar;
            cambio = true;
        }
        if self.sobre_vaciar != vaciar {
            self.sobre_vaciar = vaciar;
            cambio = true;
        }
        cambio
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        match self.zona_en(x, y) {
            Some(Zona::Cerrar) => Some(Accion::CerrarEmergente),
            Some(Zona::Vaciar) => Some(Accion::PortapapelesVaciar),
            Some(Zona::Filtro(t)) => {
                let nuevo = Filtro::desde_indice(t);
                if self.filtro != nuevo {
                    self.filtro = nuevo;
                    self.reordenar_fijadas_y_filtrar(true);
                }
                None
            }
            Some(Zona::Fijar(i)) => self
                .visibles
                .get(i)
                .map(|&idx| Accion::PortapapelesFijar(self.elementos[idx].id)),
            Some(Zona::Eliminar(i)) => self
                .visibles
                .get(i)
                .map(|&idx| Accion::PortapapelesEliminar(self.elementos[idx].id)),
            Some(Zona::Tarjeta(i)) => {
                self.elegido = i;
                self.visibles
                    .get(i)
                    .map(|&idx| Accion::PortapapelesPegar(self.elementos[idx].id))
            }
            None => {
                if self.arrastre_en(x, y) {
                    self.agarre = Some((x, y));
                }
                None
            }
        }
    }

    pub fn soltar(&mut self) -> bool {
        self.agarre.take().is_some()
    }

    pub fn desplazar(&mut self, dy: f32) -> bool {
        let destino = self.desplazamiento.destino() + dy;
        if !self.desplazar_a(destino) {
            return false;
        }
        let y = self.desplazamiento.destino();
        let colocadas = self.colocar();
        if colocadas.is_empty() {
            self.elegido = 0;
        } else if let Some(&(vis_i, cy, ch)) = colocadas.get(self.elegido) {
            if cy + ch < y || cy > y + LISTA_H {
                if let Some(&(primera, _, _)) = colocadas
                    .iter()
                    .find(|&&(_, card_y, card_h)| card_y + card_h > y && card_y < y + LISTA_H)
                {
                    self.elegido = primera;
                } else {
                    self.elegido = vis_i.min(colocadas.len() - 1);
                }
            }
        }
        self.tarjeta_senalada.señalar(None);
        true
    }

    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        use TeclaPulsada as T;
        match tecla {
            T::Escape => return Tecla::Cerrar,
            T::Intro => {
                return self
                    .visibles
                    .get(self.elegido)
                    .map(|&idx| Accion::PortapapelesPegar(self.elementos[idx].id))
                    .map_or(Tecla::Consumida, Tecla::Hacer);
            }
            T::Suprimir => {
                return self
                    .visibles
                    .get(self.elegido)
                    .map(|&idx| Accion::PortapapelesEliminar(self.elementos[idx].id))
                    .map_or(Tecla::Consumida, Tecla::Hacer);
            }
            T::Tabulador => {
                self.filtro = self.filtro.siguiente();
                self.reordenar_fijadas_y_filtrar(true);
                return Tecla::Consumida;
            }
            T::Caracter(c) if !c.is_control() && self.consulta.len() < 128 => {
                self.consulta.push(c);
                self.reordenar_fijadas_y_filtrar(true);
                return Tecla::Consumida;
            }
            T::Retroceso => {
                if self.consulta.pop().is_some() {
                    self.reordenar_fijadas_y_filtrar(true);
                }
                return Tecla::Consumida;
            }
            T::BorrarTodo => {
                if !self.consulta.is_empty() {
                    self.consulta.clear();
                    self.reordenar_fijadas_y_filtrar(true);
                }
                return Tecla::Consumida;
            }
            T::Arriba => self.elegido = self.elegido.saturating_sub(1),
            T::Abajo => {
                self.elegido = (self.elegido + 1).min(self.visibles.len().saturating_sub(1));
            }
            T::Inicio => self.elegido = 0,
            T::Fin => self.elegido = self.visibles.len().saturating_sub(1),
            T::PaginaArriba => self.elegido = self.elegido.saturating_sub(3),
            T::PaginaAbajo => {
                self.elegido = (self.elegido + 3).min(self.visibles.len().saturating_sub(1));
            }
            _ => return Tecla::Consumida,
        }
        self.asegurar_visible();
        Tecla::Consumida
    }

    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let mut nodos = Vec::new();
        nodos.push(FilaAccesible {
            id: "portapapeles:buscar".into(),
            rol: accesskit::Role::TextInput,
            nombre: t("Buscar en el historial", "Search history").into(),
            valor: self.consulta.clone(),
            rect: (MARGEN, BUSCAR_Y, INTERIOR, BUSCAR_H),
            marcada: None,
            seleccionada: false,
        });
        for (i, filtro) in Filtro::TODOS.iter().enumerate() {
            nodos.push(FilaAccesible {
                id: format!("portapapeles:filtro:{i}"),
                rol: accesskit::Role::Tab,
                nombre: filtro.etiqueta().into(),
                valor: String::new(),
                rect: (
                    MARGEN + SEG_PAD + i as f32 * SEG_TAB_ANCHO,
                    SEG_Y + SEG_PAD,
                    SEG_TAB_ANCHO,
                    SEG_H - SEG_PAD * 2.0,
                ),
                marcada: Some(self.filtro == *filtro),
                seleccionada: self.filtro == *filtro,
            });
        }
        let desp = self.desplazamiento.valor();
        for (vis_i, cy, ch) in self.colocar() {
            let y_pantalla = LISTA_Y + cy - desp;
            if y_pantalla + ch < LISTA_Y || y_pantalla > LISTA_Y + LISTA_H {
                continue;
            }
            let el = &self.elementos[self.visibles[vis_i]];
            let (nombre, valor) = match &el.contenido {
                ContenidoPortapapeles::Texto(tx) => {
                    let resumen = preview_texto(tx);
                    (
                        resumen,
                        el.origen.clone().unwrap_or_else(|| el.hora.clone()),
                    )
                }
                ContenidoPortapapeles::Imagen { formato, miniatura } => (
                    format!(
                        "{} {}×{}",
                        t("Imagen", "Image"),
                        miniatura.ancho_original,
                        miniatura.alto_original
                    ),
                    format!("{formato} · {}", el.hora),
                ),
            };
            nodos.push(FilaAccesible {
                id: format!("portapapeles:item:{}", el.id),
                rol: accesskit::Role::ListItem,
                nombre,
                valor,
                rect: (MARGEN, y_pantalla, INTERIOR, ch),
                marcada: Some(el.fijada),
                seleccionada: vis_i == self.elegido,
            });
        }
        if !self.elementos.is_empty() {
            let w = self.ancho_vaciar();
            let x0 = ANCHO - MARGEN - BTN_LADO - 6.0 - w;
            nodos.push(FilaAccesible {
                id: "portapapeles:vaciar".into(),
                rol: accesskit::Role::Button,
                nombre: t("Vaciar historial", "Clear history").into(),
                valor: String::new(),
                rect: (x0, CAB_Y, w, CAB_H),
                marcada: None,
                seleccionada: false,
            });
        }
        nodos.push(FilaAccesible {
            id: "portapapeles:cerrar".into(),
            rol: accesskit::Role::Button,
            nombre: t("Cerrar", "Close").into(),
            valor: String::new(),
            rect: (ANCHO - MARGEN - BTN_LADO, CAB_Y, BTN_LADO, BTN_LADO),
            marcada: None,
            seleccionada: false,
        });
        nodos
    }

    pub fn view(&self) -> PanelElement<'_> {
        let vaciar: PanelElement<'_> = if self.elementos.is_empty() {
            Space::new().width(0.0).into()
        } else {
            let fondo_vaciar = if self.sobre_vaciar {
                tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.06)
            } else {
                tema::superficie_en_tarjeta()
            };
            container(
                text(t("Vaciar", "Clear"))
                    .size(12)
                    .font(control::peso(Weight::Semibold))
                    .color(tema::texto()),
            )
            .width(Length::Fixed(self.ancho_vaciar()))
            .height(Length::Fixed(CAB_H))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_| container::Style {
                background: Some(fondo_vaciar.into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        };

        let fondo_cerrar = self.sobre_cerrar.then(tema::superficie_en_tarjeta);
        let cerrar = container(icono::ver_teñido_propio(
            &icono::propio("cerrar").expect("cerrar es un icono integrado"),
            14.0,
            if self.sobre_cerrar {
                tema::texto()
            } else {
                tema::texto2()
            },
        ))
        .center(BTN_LADO)
        .style(move |_| container::Style {
            background: fondo_cerrar.map(Into::into),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let titulo = row![
            text(t("Portapapeles", "Clipboard"))
                .size(15)
                .font(control::peso(Weight::Semibold))
                .color(tema::texto()),
            Space::new().width(Length::Fill),
            vaciar,
            Space::new().width(if self.elementos.is_empty() { 0.0 } else { 6.0 }),
            cerrar,
        ]
        .height(CAB_H)
        .align_y(Vertical::Center);

        let consulta = if self.consulta.is_empty() {
            t("Buscar en el historial", "Search history").to_string()
        } else {
            super::recortar_texto(&self.consulta, INTERIOR - 56.0, 14.0)
        };
        let campo_fondo = tema::superficie_en_tarjeta();
        let acento_campo = tema::acento_en(campo_fondo);
        let campo = container(
            row![
                icono::ver_teñido_propio(
                    &icono::propio("buscar").expect("buscar es un icono integrado"),
                    16.0,
                    acento_campo,
                ),
                text(consulta).size(14).color(if self.consulta.is_empty() {
                    tema::texto2()
                } else {
                    tema::texto()
                }),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        )
        .padding([0, 12])
        .width(INTERIOR)
        .height(BUSCAR_H)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(campo_fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                width: 1.0,
                color: tema::alfa(acento_campo, 0.45),
            },
            ..Default::default()
        });

        let mut tabs = row![];
        for (i, filtro) in Filtro::TODOS.iter().enumerate() {
            let activo = self.filtro == *filtro;
            let hover = self.tab_senalada.intensidad(i);
            let (fondo, color, peso) = if activo {
                (
                    Some(tema::acento()),
                    tema::sobre_acento(),
                    Weight::Semibold,
                )
            } else if hover > 0.0 {
                (
                    Some(tema::mezclar(
                        tema::superficie_en_tarjeta(),
                        tema::tinta(),
                        0.05 * hover,
                    )),
                    tema::texto2(),
                    Weight::Medium,
                )
            } else {
                (None, tema::texto2(), Weight::Medium)
            };
            tabs = tabs.push(
                container(
                    text(filtro.etiqueta())
                        .size(13)
                        .font(control::peso(peso))
                        .color(color),
                )
                .width(SEG_TAB_ANCHO)
                .height(SEG_H - SEG_PAD * 2.0)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: fondo.map(Into::into),
                    border: Border {
                        radius: tema::R_BOTON_PEQUENO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        let segmentado = container(tabs)
            .padding(SEG_PAD)
            .width(INTERIOR)
            .height(SEG_H)
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let etiqueta_sub = if !self.consulta.trim().is_empty() {
            t("Resultados", "Results")
        } else {
            match self.filtro {
                Filtro::Todo => t("Recientes", "Recent"),
                Filtro::Texto => t("Textos", "Text"),
                Filtro::Fotos => t("Fotos", "Images"),
            }
        };
        let cabecera = row![
            text(etiqueta_sub).size(12).color(tema::texto2()),
            Space::new().width(Length::Fill),
            text(self.visibles.len().to_string())
                .size(11)
                .color(tema::texto2()),
        ]
        .height(SUB_H)
        .align_y(Vertical::Center);

        let lista: PanelElement<'_> = if self.visibles.is_empty() {
            let (titulo_vacio, sub_vacio) = if self.elementos.is_empty() {
                (
                    t("El portapapeles está vacío", "Clipboard is empty"),
                    t(
                        "Copia texto o imágenes para verlos aquí",
                        "Copy text or images to see them here",
                    ),
                )
            } else {
                (
                    t("Sin resultados", "No results"),
                    t(
                        "Prueba otro filtro o término de búsqueda",
                        "Try another filter or search term",
                    ),
                )
            };
            container(
                column![
                    icono::ver_teñido_propio(
                        &icono::propio("copiar").expect("copiar es un icono integrado"),
                        28.0,
                        tema::texto2(),
                    ),
                    text(titulo_vacio)
                        .size(14)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto()),
                    text(sub_vacio).size(11.5).color(tema::texto2()),
                ]
                .spacing(6)
                .align_x(Horizontal::Center),
            )
            .width(INTERIOR)
            .height(LISTA_H)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
        } else {
            let desp = self.desplazamiento.valor();
            let colocadas = self.colocar();
            let primera_idx = colocadas
                .iter()
                .position(|&(_, cy, ch)| cy + ch > desp)
                .unwrap_or(0);
            let offset_inicial = (desp - colocadas[primera_idx].1).max(0.0);

            let mut col = column![].spacing(CARD_GAP);
            for &(vis_i, cy, _) in &colocadas[primera_idx..] {
                if cy >= desp + LISTA_H {
                    break;
                }
                col = col.push(self.vista_tarjeta(vis_i));
            }
            crate::recorte::Recorte::new(col, INTERIOR, LISTA_H, offset_inicial).into()
        };

        let pie = row![
            text(t("↑↓ navegar · Tab filtro", "↑↓ navigate · Tab filter"))
                .size(11)
                .color(tema::texto2()),
            Space::new().width(Length::Fill),
            text(t("Intro pegar · Supr borrar", "Enter paste · Del remove"))
                .size(11)
                .color(tema::texto2()),
        ]
        .height(PIE_H)
        .align_y(Vertical::Center);

        control::tarjeta(
            column![
                titulo,
                Space::new().height(8),
                campo,
                Space::new().height(8),
                segmentado,
                Space::new().height(8),
                cabecera,
                Space::new().height(6),
                lista,
                Space::new().height(6),
                pie,
            ]
            .into(),
            ANCHO,
            MARGEN,
        )
    }

    fn vista_tarjeta(&self, vis_i: usize) -> PanelElement<'_> {
        let el = &self.elementos[self.visibles[vis_i]];
        let elegida = vis_i == self.elegido;
        let hover = self.tarjeta_senalada.intensidad(vis_i);
        let fondo = if elegida {
            tema::acento_suave_en_tarjeta()
        } else if hover > 0.0 {
            tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.05 * hover)
        } else {
            tema::superficie_en_tarjeta()
        };
        let acento_tinta = tema::acento_en(fondo);
        let borde_color = if elegida {
            tema::acento_en(tema::card())
        } else {
            Color::TRANSPARENT
        };

        let botones = self.vista_botones(vis_i, el.fijada, fondo);
        let meta = self.vista_meta(el, fondo);

        let cuerpo: PanelElement<'_> = match &el.contenido {
            ContenidoPortapapeles::Texto(tx) => {
                let ancho_texto = INTERIOR - 12.0 - 12.0 - BTN_LADO * 2.0 - 8.0;
                let preview = preview_texto(tx);
                let lineas = crate::escritorio::lineas(&preview, ancho_texto, 13.5, 2);
                let mut bloque = column![].spacing(2);
                for linea in lineas {
                    bloque = bloque.push(text(linea).size(13.5).color(tema::texto()));
                }
                container(
                    column![
                        row![
                            container(bloque).width(Length::Fill),
                            botones,
                        ]
                        .align_y(Vertical::Top),
                        Space::new().height(Length::Fill),
                        meta,
                    ]
                    .height(Length::Fill),
                )
                .padding(Padding {
                    top: 6.0,
                    right: 6.0,
                    bottom: 8.0,
                    left: 12.0,
                })
                .width(INTERIOR)
                .height(ALTO_TEXTO)
                .into()
            }
            ContenidoPortapapeles::Imagen { miniatura, .. } => {
                let caja_w = INTERIOR - 16.0;
                let handle = self.handles.get(&el.id).cloned().unwrap_or_else(|| {
                    imagen::Handle::from_rgba(
                        miniatura.ancho,
                        miniatura.alto,
                        (*miniatura.rgba).clone(),
                    )
                });
                let caja_img = container(
                    imagen(handle)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .content_fit(ContentFit::Contain),
                )
                .width(caja_w)
                .height(MINI_CAJA_H)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(|_| container::Style {
                    background: Some(tema::elevado().into()),
                    border: Border {
                        radius: tema::R_BOTON_PEQUENO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
                container(
                    column![
                        caja_img,
                        Space::new().height(4),
                        row![
                            container(meta).width(Length::Fill).padding([0, 4]),
                            botones,
                        ]
                        .height(BTN_LADO)
                        .align_y(Vertical::Center),
                    ],
                )
                .padding(Padding {
                    top: 8.0,
                    right: 6.0,
                    bottom: 6.0,
                    left: 8.0,
                })
                .width(INTERIOR)
                .height(ALTO_IMAGEN)
                .into()
            }
        };

        let alto = Self::alto_tarjeta(el);
        let contenido: PanelElement<'_> = if elegida {
            // Marca perceptible sin color (HIG §2 / §11) junto al borde acentuado.
            let barra = container(Space::new())
                .width(3.0)
                .height(alto - 20.0)
                .style(move |_| container::Style {
                    background: Some(acento_tinta.into()),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            stack![
                cuerpo,
                container(barra).padding(Padding {
                    top: 10.0,
                    left: 3.0,
                    ..Padding::ZERO
                }),
            ]
            .into()
        } else {
            cuerpo
        };

        container(contenido)
            .width(INTERIOR)
            .height(alto)
            .style(move |_| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_ITEM_POPOVER.into(),
                    width: if elegida { 1.5 } else { 0.0 },
                    color: borde_color,
                },
                ..Default::default()
            })
            .into()
    }

    fn vista_botones(&self, vis_i: usize, fijada: bool, fondo_tarjeta: Color) -> PanelElement<'_> {
        let hover_pin = self.fijar_senalado.intensidad(vis_i);
        let hover_del = self.eliminar_senalado.intensidad(vis_i);
        let fondo_pin = (hover_pin > 0.0)
            .then(|| tema::mezclar(fondo_tarjeta, tema::tinta(), 0.08 * hover_pin));
        let fondo_del = (hover_del > 0.0)
            .then(|| tema::mezclar(fondo_tarjeta, tema::rojo_en(fondo_tarjeta), 0.14 * hover_del));
        let nombre_pin = if fijada { "fijar-anclado" } else { "fijar" };
        let color_pin = if fijada {
            tema::acento_en(fondo_tarjeta)
        } else if hover_pin > 0.0 {
            tema::texto()
        } else {
            tema::texto2()
        };
        let color_del = if hover_del > 0.0 {
            tema::rojo_en(fondo_tarjeta)
        } else {
            tema::texto2()
        };

        let pin = container(icono::ver_teñido_propio(
            &icono::propio(nombre_pin).expect("icono de fijar integrado"),
            15.0,
            color_pin,
        ))
        .center(BTN_LADO)
        .style(move |_| container::Style {
            background: fondo_pin.map(Into::into),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let del = container(icono::ver_teñido_propio(
            &icono::propio("papelera").expect("icono de papelera integrado"),
            15.0,
            color_del,
        ))
        .center(BTN_LADO)
        .style(move |_| container::Style {
            background: fondo_del.map(Into::into),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        row![pin, Space::new().width(2), del].into()
    }

    fn vista_meta(&self, el: &ElementoPortapapeles, fondo_tarjeta: Color) -> PanelElement<'_> {
        let mut partes = Vec::new();
        if let Some(origen) = el.origen.as_deref().filter(|s| !s.is_empty()) {
            partes.push(origen.to_string());
        }
        match &el.contenido {
            ContenidoPortapapeles::Texto(tx) => {
                partes.push(el.hora.clone());
                let chars = tx.chars().count();
                let unidad = if chars == 1 {
                    t("car.", "char")
                } else {
                    t("car.", "chars")
                };
                partes.push(format!("{chars} {unidad}"));
            }
            ContenidoPortapapeles::Imagen { formato, miniatura } => {
                partes.push(format!(
                    "{}×{} · {}",
                    miniatura.ancho_original, miniatura.alto_original, formato
                ));
                partes.push(el.hora.clone());
            }
        }
        let max_w = match &el.contenido {
            ContenidoPortapapeles::Texto(_) => INTERIOR - 24.0 - if el.fijada { 58.0 } else { 0.0 },
            ContenidoPortapapeles::Imagen { .. } => {
                INTERIOR - 24.0 - BTN_LADO * 2.0 - if el.fijada { 20.0 } else { 0.0 }
            }
        };
        let texto_meta = super::recortar_texto(&partes.join(" · "), max_w.max(60.0), 11.0);

        let mut fila = row![].spacing(4).align_y(Vertical::Center);
        if el.fijada {
            fila = fila.push(icono::ver_teñido_propio(
                &icono::propio("fijar-anclado").expect("fijar-anclado integrado"),
                11.0,
                tema::acento_en(fondo_tarjeta),
            ));
            if matches!(el.contenido, ContenidoPortapapeles::Texto(_)) {
                fila = fila.push(
                    text(t("Fijado ·", "Pinned ·"))
                        .size(11)
                        .font(control::peso(Weight::Medium))
                        .color(tema::acento_en(fondo_tarjeta)),
                );
            }
        }
        fila = fila.push(text(texto_meta).size(11).color(tema::texto2()));
        fila.into()
    }
}

fn preview_texto(texto: &str) -> String {
    texto
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
    use image::{ExtendedColorType, ImageEncoder as _};

    fn png_prueba(w: u32, h: u32) -> Vec<u8> {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                rgba[i] = ((x * 255) / w.max(1)) as u8;
                rgba[i + 1] = ((y * 255) / h.max(1)) as u8;
                rgba[i + 2] = 200;
                rgba[i + 3] = 255;
            }
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&rgba, w, h, ExtendedColorType::Rgba8)
            .expect("png válido");
        out
    }

    fn item_texto(id: u64, texto: &str, fijada: bool) -> ElementoPortapapeles {
        ElementoPortapapeles {
            id,
            fijada,
            hora: "14:20".into(),
            origen: Some("Terminal".into()),
            contenido: ContenidoPortapapeles::Texto(texto.into()),
        }
    }

    fn item_imagen(id: u64, w: u32, h: u32, fijada: bool) -> ElementoPortapapeles {
        let bytes = png_prueba(w, h);
        let miniatura = miniatura_de_imagen(&bytes).expect("miniatura");
        ElementoPortapapeles {
            id,
            fijada,
            hora: "14:25".into(),
            origen: Some("Captura".into()),
            contenido: ContenidoPortapapeles::Imagen {
                formato: "PNG".into(),
                miniatura,
            },
        }
    }

    #[test]
    fn miniatura_escala_proporcionalmente_y_redondea_esquinas() {
        let bytes = png_prueba(1280, 720);
        let mini = miniatura_de_imagen(&bytes).expect("miniatura generada");
        assert_eq!(mini.ancho_original, 1280);
        assert_eq!(mini.alto_original, 720);
        assert!(mini.ancho <= MINI_MAX_W && mini.alto <= MINI_MAX_H);
        assert_eq!(mini.rgba.len(), (mini.ancho * mini.alto * 4) as usize);
        // Esquina superior izquierda recortada por el redondeo.
        assert_eq!(mini.rgba[3], 0);
        // Centro opaco.
        let cx = mini.ancho / 2;
        let cy = mini.alto / 2;
        let centro = ((cy * mini.ancho + cx) * 4 + 3) as usize;
        assert_eq!(mini.rgba[centro], 255);
    }

    #[test]
    fn filtro_busqueda_y_fijadas_funcionan() {
        let items = vec![
            item_texto(1, "Primer texto", false),
            item_imagen(2, 400, 300, false),
            item_texto(3, "Dirección con tilde: Ávila", true),
        ];
        let mut p = Portapapeles::new(50.0, 50.0, items);
        // Fijada primero.
        assert_eq!(p.elementos[p.visibles[0]].id, 3);

        // Tab cambia a Texto, luego a Fotos, luego a Todo.
        p.tecla(TeclaPulsada::Tabulador);
        assert_eq!(p.filtro, Filtro::Texto);
        assert_eq!(p.visibles.len(), 2);

        p.tecla(TeclaPulsada::Tabulador);
        assert_eq!(p.filtro, Filtro::Fotos);
        assert_eq!(p.visibles.len(), 1);
        assert_eq!(p.elementos[p.visibles[0]].id, 2);

        p.tecla(TeclaPulsada::Tabulador);
        assert_eq!(p.filtro, Filtro::Todo);

        // Búsqueda sin tildes.
        for c in "avila".chars() {
            p.tecla(TeclaPulsada::Caracter(c));
        }
        assert_eq!(p.visibles.len(), 1);
        assert_eq!(p.elementos[p.visibles[0]].id, 3);
    }

    #[test]
    fn teclado_navega_pega_borra_y_cierra() {
        let items = vec![item_texto(10, "Uno", false), item_texto(20, "Dos", false)];
        let mut p = Portapapeles::new(0.0, 0.0, items);
        p.tecla(TeclaPulsada::Abajo);
        assert_eq!(p.elegido, 1);
        assert!(matches!(
            p.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::PortapapelesPegar(20))
        ));
        assert!(matches!(
            p.tecla(TeclaPulsada::Suprimir),
            Tecla::Hacer(Accion::PortapapelesEliminar(20))
        ));
        assert!(matches!(p.tecla(TeclaPulsada::Escape), Tecla::Cerrar));
    }

    #[test]
    fn clics_en_botones_tarjeta_y_arrastre() {
        let items = vec![item_texto(1, "Texto", false), item_imagen(2, 200, 120, false)];
        let mut p = Portapapeles::new(100.0, 100.0, items);

        // Clic en cerrar.
        assert_eq!(
            p.pulsar(ANCHO - MARGEN - 10.0, CAB_Y + 10.0),
            Some(Accion::CerrarEmergente)
        );
        // Clic en vaciar.
        let vaciar_x = ANCHO - MARGEN - BTN_LADO - 12.0;
        assert_eq!(
            p.pulsar(vaciar_x, CAB_Y + 10.0),
            Some(Accion::PortapapelesVaciar)
        );

        // Clic en fijar y eliminar de la primera tarjeta (texto).
        let del_x = MARGEN + INTERIOR - 6.0 - 10.0;
        let pin_x = MARGEN + INTERIOR - 6.0 - BTN_LADO - 12.0;
        assert_eq!(
            p.pulsar(pin_x, LISTA_Y + 12.0),
            Some(Accion::PortapapelesFijar(1))
        );
        assert_eq!(
            p.pulsar(del_x, LISTA_Y + 12.0),
            Some(Accion::PortapapelesEliminar(1))
        );

        // Clic en el cuerpo de la segunda tarjeta (foto).
        let foto_y = LISTA_Y + ALTO_TEXTO + CARD_GAP + 20.0;
        assert_eq!(
            p.pulsar(MARGEN + 20.0, foto_y),
            Some(Accion::PortapapelesPegar(2))
        );

        // Arrastre por título.
        assert_eq!(p.pulsar(MARGEN + 20.0, CAB_Y + 10.0), None);
        assert!(p.agarrado());
        assert!(p.puntero(Some((MARGEN + 40.0, CAB_Y + 25.0))));
        assert!(p.soltar());
        assert_eq!(p.ancla(), Ancla::Punto { x: 120.0, y: 115.0 });
        p.recortar_ancla(8.0, 8.0);
        assert_eq!(p.ancla(), Ancla::Punto { x: 8.0, y: 8.0 });
    }

    #[test]
    fn accesible_expone_controles_y_elementos() {
        let items = vec![item_texto(7, "Hola mundo", true)];
        let p = Portapapeles::new(0.0, 0.0, items);
        let nodos = p.accesible();
        assert!(nodos.iter().any(|n| n.id == "portapapeles:buscar"));
        assert!(nodos.iter().any(|n| n.id == "portapapeles:item:7" && n.seleccionada));
        assert!(nodos.iter().any(|n| n.id == "portapapeles:vaciar"));
        assert!(nodos.iter().any(|n| n.id == "portapapeles:cerrar"));
    }
}
