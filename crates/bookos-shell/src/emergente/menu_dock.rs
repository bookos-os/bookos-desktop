//! Los menús contextuales del escritorio y del dock, y el menú de ventana.
//!
//! Todos comparten las filas del menú del logo (`menu::fila_con`): el mismo
//! control, el mismo hover en acento sólido. Duplicarlas habría dejado menús
//! que se parecen pero no se comportan igual.
//!
//! Quien abre el menú decide **qué** ofrece y lo entrega ya armado como
//! [`Entrada`]s; aquí solo se decide cómo se dibuja, cómo se recorre con el
//! ratón y el teclado, y dónde caen sus submenús. Así el escritorio, que sabe
//! qué hay seleccionado, y el dock, que sabe qué ventanas tiene cada app, no
//! tienen que conocer la geometría.
//!
//! ## Repintar no es entrar
//!
//! El menú no lleva animación propia de apertura: la pone el compositor con el
//! muelle de todas las emergentes, y ese muelle solo arranca cuando la
//! superficie es **nueva**. Por eso el tamaño se fija al construir —reservando
//! el sitio de cualquier submenú, abierto o no—: si la superficie cambiara de
//! tamaño al desplegar «Ordenar por ›», se rehará y el menú volvería a «salir».

use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

use super::menu::{
    CHEVRON, DIVISOR, Delante, FILA, HUECO_ATAJO, HUECO_ICONO, ICONO, MARGEN, RELLENO, ancho_atajo,
    divisor, fila_con,
};
use super::{Ancla, Tecla};

/// Iguales al margen lateral de las filas, como en el menú del logo: con 4
/// arriba y 5 a los lados el realce de la primera fila no quedaba concéntrico
/// con la esquina.
const BORDE: f32 = MARGEN;

/// Hueco entre el menú y su submenú. Cuenta como zona del submenú al mover el
/// puntero: si no, cruzarlo en diagonal cerraba el submenú justo antes de
/// llegar.
const HUECO_SUBMENU: f32 = 2.0;

/// Alto de un título de grupo («Ventanas», «Tamaño de los iconos»): 11/600 en
/// mayúsculas del HIG §9, con 7 arriba y 4 abajo como en la maqueta.
const TITULO: f32 = 24.0;
/// Cabecera: miniatura de 36, 5 arriba y 9 abajo hasta la fila de acciones.
const CABECERA: f32 = 50.0;
const MINI: f32 = 36.0;
/// Botones rápidos: 48 de alto (zona táctil del HIG §11 de sobra) y debajo una
/// raya con 5 de aire a cada lado, que separa las acciones de un toque de la
/// lista.
const RAPIDO: f32 = 48.0;
const RAPIDAS: f32 = RAPIDO + 5.0 + 1.0 + 5.0;
/// Fila de colores: las muestras son de 16 pero cada una ocupa 28 de zona
/// activa, el mínimo con puntero del HIG §11.
const COLORES: f32 = 34.0;
const MUESTRA: f32 = 16.0;
const ZONA_MUESTRA: f32 = 28.0;

/// Ancho mínimo con cabecera o botones rápidos: por debajo «Renombrar» toca los
/// bordes de su botón (se vio en la maqueta con 248).
const ANCHO_RICO: f32 = 272.0;
/// Sin ellos, lo que pida la fila más larga, pero no menos que esto: un menú
/// de dos palabras más estrecho que el puntero se lee como un globo de ayuda.
const ANCHO_MIN: f32 = 180.0;

/// Qué lleva delante una fila además (o en vez) de su icono.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marca {
    Ninguna,
    /// Se puede poner y quitar: «Mostrar iconos», «Abrir al iniciar sesión».
    Casilla(bool),
    /// Una de un grupo excluyente: «Ordenar por › Nombre».
    Opcion(bool),
}

/// Una fila del menú.
#[derive(Clone)]
pub struct Fila {
    pub etiqueta: String,
    pub icono: Option<&'static str>,
    pub atajo: Option<&'static str>,
    pub marca: Marca,
    /// Destructiva: roja, y su realce también (HIG §2).
    pub peligro: bool,
    /// `None` con submenú: la fila solo lo despliega.
    pub accion: Option<Accion>,
    pub sub: Vec<Entrada>,
}

impl Fila {
    // añadido en la 0.2.3
    pub fn new(etiqueta: impl Into<String>, icono: &'static str, accion: Accion) -> Self {
        Self {
            etiqueta: etiqueta.into(),
            icono: Some(icono),
            atajo: None,
            marca: Marca::Ninguna,
            peligro: false,
            accion: Some(accion),
            sub: Vec::new(),
        }
    }

    /// Una fila que despliega `sub` a su derecha.
    // añadido en la 0.2.3
    pub fn submenu(etiqueta: impl Into<String>, icono: &'static str, sub: Vec<Entrada>) -> Self {
        Self {
            etiqueta: etiqueta.into(),
            icono: Some(icono),
            atajo: None,
            marca: Marca::Ninguna,
            peligro: false,
            accion: None,
            sub,
        }
    }

    /// Una fila sin icono que lleva una marca en su lugar.
    // añadido en la 0.2.3
    pub fn marcada(etiqueta: impl Into<String>, marca: Marca, accion: Accion) -> Self {
        Self {
            etiqueta: etiqueta.into(),
            icono: None,
            atajo: None,
            marca,
            peligro: false,
            accion: Some(accion),
            sub: Vec::new(),
        }
    }

    // añadido en la 0.2.3
    pub fn atajo(mut self, atajo: &'static str) -> Self {
        self.atajo = Some(atajo);
        self
    }

    // añadido en la 0.2.3
    pub fn peligro(mut self) -> Self {
        self.peligro = true;
        self
    }

    fn delante(&self) -> Option<Delante<'static>> {
        match (self.marca, self.icono) {
            (Marca::Casilla(m), _) => Some(Delante::Casilla(m)),
            (Marca::Opcion(m), _) => Some(Delante::Opcion(m)),
            (Marca::Ninguna, Some(i)) => Some(Delante::Icono(i)),
            (Marca::Ninguna, None) => None,
        }
    }

    /// El ancho que pide, con margen, relleno, icono, atajo y chevron.
    fn ancho(&self) -> f32 {
        (MARGEN + RELLENO) * 2.0
            + self.delante().map_or(0.0, |_| ICONO + HUECO_ICONO)
            + crate::widget::ancho_de(&self.etiqueta, tema::T_CUERPO)
            + self.atajo.map_or(0.0, |a| HUECO_ATAJO + ancho_atajo(a))
            + if self.sub.is_empty() {
                0.0
            } else {
                HUECO_ATAJO + CHEVRON
            }
    }
}

/// Un botón de la fila de acciones rápidas de arriba.
#[derive(Clone)]
pub struct Rapida {
    pub etiqueta: String,
    pub icono: &'static str,
    pub accion: Accion,
}

/// Una pieza del menú, de arriba abajo.
#[derive(Clone)]
pub enum Entrada {
    Fila(Fila),
    Divisor,
    /// Título de grupo, en mayúsculas.
    Titulo(String),
    /// Sobre qué se actúa: icono, nombre y una línea de detalle.
    Cabecera {
        icono: Option<Icono>,
        nombre: String,
        detalle: String,
    },
    /// Botones de un toque con icono y palabra (HIG §11: nunca solo icono).
    Rapidas(Vec<Rapida>),
    /// Las muestras de color de carpeta, con «+» para el selector. `elegido`
    /// es el color que comparten las carpetas seleccionadas, si lo comparten.
    Colores {
        elegido: Option<String>,
    },
}

impl Entrada {
    fn alto(&self, fila: f32) -> f32 {
        match self {
            Self::Fila(_) => fila,
            Self::Divisor => DIVISOR,
            Self::Titulo(_) => TITULO,
            Self::Cabecera { .. } => CABECERA,
            Self::Rapidas(_) => RAPIDAS,
            Self::Colores { .. } => COLORES,
        }
    }

    fn ancho(&self) -> f32 {
        match self {
            Self::Fila(f) => f.ancho(),
            Self::Titulo(t) => {
                (MARGEN + RELLENO) * 2.0
                    + crate::widget::ancho_de(&t.to_uppercase(), tema::T_PEQUENO - 1.0)
            }
            Self::Cabecera { .. } | Self::Rapidas(_) | Self::Colores { .. } => ANCHO_RICO,
            Self::Divisor => 0.0,
        }
    }
}

/// Algo que se puede señalar y pulsar dentro del menú principal: una fila, o
/// un botón de un grupo horizontal (acciones rápidas, muestras de color).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Objetivo {
    entrada: usize,
    /// Posición dentro de un grupo horizontal; `0` en una fila.
    k: usize,
}

/// Dónde se dibuja el submenú respecto a su fila madre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lado {
    /// Su primera fila a la altura de la madre: lo de siempre.
    Abajo,
    /// Su última fila a la altura de la madre. Lo usa el dock: el menú nace
    /// pegado a él, y un submenú hacia abajo se saldría por debajo del dock.
    Arriba,
}

pub struct MenuDock {
    ventana: bool,
    entradas: Vec<Entrada>,
    objetivos: Vec<Objetivo>,
    /// Ancho del menú principal y del submenú más ancho.
    ancho: f32,
    ancho_sub: f32,
    /// Dónde empieza el menú principal dentro de la superficie. El dock
    /// reserva el sitio del submenú **a los dos lados**: el compositor centra
    /// la superficie sobre el icono, y reservando solo a la derecha el menú
    /// salía corrido a la izquierda.
    origen: (f32, f32),
    tamaño: (f32, f32),
    lado: Lado,
    señalado: tema::Realce,
    /// En los grupos horizontales el realce lleva el objetivo entero; para
    /// las filas basta con `señalado`, que guarda el índice en `objetivos`.
    anclaje: Ancla,
    /// La fila que tiene el submenú desplegado.
    abierto: Option<usize>,
    señalada_sub: tema::Realce,
    /// Las flechas mueven la lista del submenú y no la principal.
    foco_sub: bool,
}

impl MenuDock {
    // 0.1.0
    pub fn es_ventana(&self) -> bool {
        self.ventana
    }

    /// El menú de ventana: centrado y con filas de 48 sin icono, pensado para
    /// leerse en medio de la pantalla.
    // 0.1.0
    // modified 0.2.3 -- sobre el modelo de entradas común
    pub fn ventana(opciones: Vec<(String, Accion)>) -> Self {
        let entradas = opciones
            .into_iter()
            .map(|(etiqueta, accion)| {
                Entrada::Fila(Fila {
                    icono: None,
                    ..Fila::new(etiqueta, "", accion)
                })
            })
            .collect();
        let mut menu = Self::construir(entradas, Ancla::Centrada, Lado::Abajo, false);
        menu.ventana = true;
        menu.ancho = 310.0;
        menu.recalcular();
        menu
    }

    /// El clic derecho sobre el escritorio, con la esquina en `punto`.
    // 0.1.1
    // modified 0.2.3 -- recibe las entradas armadas por el escritorio
    pub fn escritorio(entradas: Vec<Entrada>, punto: (f32, f32)) -> Self {
        let (x, y) = punto;
        Self::construir(entradas, Ancla::Punto { x, y }, Lado::Abajo, false)
    }

    /// El clic derecho sobre un icono del dock, centrado sobre él. `x` es su
    /// centro en lógicos relativos al dock.
    // 0.1.0
    // modified 0.2.3 -- recibe las entradas armadas por el dock
    pub fn dock(entradas: Vec<Entrada>, x: f32) -> Self {
        Self::construir(entradas, Ancla::SobreElDock { x }, Lado::Arriba, true)
    }

    fn construir(entradas: Vec<Entrada>, anclaje: Ancla, lado: Lado, simetrico: bool) -> Self {
        let objetivos = entradas
            .iter()
            .enumerate()
            .flat_map(|(i, e)| {
                let n = match e {
                    Entrada::Fila(_) => 1,
                    Entrada::Rapidas(r) => r.len(),
                    Entrada::Colores { .. } => crate::escritorio::MUESTRAS.len() + 1,
                    _ => 0,
                };
                (0..n).map(move |k| Objetivo { entrada: i, k })
            })
            .collect();
        let mut menu = Self {
            ventana: false,
            entradas,
            objetivos,
            ancho: 0.0,
            ancho_sub: 0.0,
            origen: (0.0, 0.0),
            tamaño: (0.0, 0.0),
            lado,
            señalado: tema::Realce::nuevo(),
            anclaje,
            abierto: None,
            señalada_sub: tema::Realce::nuevo(),
            foco_sub: false,
        };
        let cabe = |es: &[Entrada]| es.iter().map(Entrada::ancho).fold(0.0, f32::max);
        // Con cabecera, botones o colores el menú es «rico» y tiene su mínimo;
        // si no, el de una lista corta.
        menu.ancho = cabe(&menu.entradas).max(ANCHO_MIN).ceil();
        menu.ancho_sub = menu
            .entradas
            .iter()
            .filter_map(|e| match e {
                Entrada::Fila(f) if !f.sub.is_empty() => Some(cabe(&f.sub).max(ANCHO_MIN)),
                _ => None,
            })
            .fold(0.0, f32::max)
            .ceil();
        if simetrico && menu.ancho_sub > 0.0 {
            menu.origen.0 = menu.ancho_sub + HUECO_SUBMENU;
        }
        menu.recalcular();
        menu
    }

    /// Fija el tamaño de la superficie y dónde va el menú dentro de ella,
    /// reservando el sitio de **todos** los submenús aunque estén cerrados:
    /// crecer al abrir uno movería el menú bajo el puntero y rearrancaría la
    /// entrada (ver la cabecera del módulo).
    fn recalcular(&mut self) {
        let alto = self.alto_principal();
        let (mut arriba, mut abajo) = (0.0_f32, alto);
        for i in self.con_submenu() {
            let (y, h) = self.caja_sub(i);
            arriba = arriba.min(y);
            abajo = abajo.max(y + h);
        }
        self.origen.1 = -arriba;
        let derecha = if self.ancho_sub > 0.0 {
            HUECO_SUBMENU + self.ancho_sub
        } else {
            0.0
        };
        self.tamaño = (self.origen.0 + self.ancho + derecha, abajo - arriba);
    }

    fn alto_fila(&self) -> f32 {
        if self.ventana { 48.0 } else { FILA }
    }

    fn alto_principal(&self) -> f32 {
        let filas: f32 = self.entradas.iter().map(|e| e.alto(self.alto_fila())).sum();
        filas + BORDE * 2.0
    }

    /// La `y` donde empieza la entrada `i`, relativa al menú principal.
    fn y_de(&self, i: usize) -> f32 {
        BORDE
            + self.entradas[..i]
                .iter()
                .map(|e| e.alto(self.alto_fila()))
                .sum::<f32>()
    }

    fn con_submenu(&self) -> impl Iterator<Item = usize> + '_ {
        self.entradas
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Entrada::Fila(f) if !f.sub.is_empty()))
            .map(|(i, _)| i)
    }

    fn sub_de(&self, i: usize) -> &[Entrada] {
        match self.entradas.get(i) {
            Some(Entrada::Fila(f)) => &f.sub,
            _ => &[],
        }
    }

    /// `y` y alto del submenú de la fila `i`, relativos al menú principal.
    fn caja_sub(&self, i: usize) -> (f32, f32) {
        let alto: f32 = self.sub_de(i).iter().map(|e| e.alto(FILA)).sum::<f32>() + BORDE * 2.0;
        let y = match self.lado {
            // Su primera fila a la altura de la madre.
            Lado::Abajo => self.y_de(i) - BORDE,
            // Su última fila a la altura de la madre.
            Lado::Arriba => self.y_de(i) + FILA + BORDE - alto,
        };
        (y, alto)
    }

    /// Las filas pulsables del submenú `i`, con su `y` dentro de él.
    fn filas_sub(&self, i: usize) -> Vec<(f32, &Fila)> {
        let mut y = BORDE;
        let mut filas = Vec::new();
        for e in self.sub_de(i) {
            if let Entrada::Fila(f) = e {
                filas.push((y, f));
            }
            y += e.alto(FILA);
        }
        filas
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        self.tamaño
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        self.anclaje
    }

    /// Pasa un punto de la superficie a coordenadas del menú principal.
    fn local(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.origen.0, y - self.origen.1)
    }

    /// Qué fila del submenú abierto cae en el punto (ya local). Cuenta el
    /// hueco con el menú, pero solo a la altura del propio submenú.
    fn fila_sub_en(&self, x: f32, y: f32) -> Option<usize> {
        let i = self.abierto?;
        if x < self.ancho || x > self.ancho + HUECO_SUBMENU + self.ancho_sub {
            return None;
        }
        let (arriba, alto) = self.caja_sub(i);
        if y < arriba || y > arriba + alto {
            return None;
        }
        self.filas_sub(i)
            .iter()
            .position(|(fy, _)| y - arriba >= *fy && y - arriba < fy + FILA)
    }

    /// El objetivo del menú principal bajo el punto (ya local).
    fn objetivo_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho {
            return None;
        }
        let i = (0..self.entradas.len()).find(|&i| {
            let top = self.y_de(i);
            y >= top && y < top + self.entradas[i].alto(self.alto_fila())
        })?;
        let k = match &self.entradas[i] {
            Entrada::Fila(_) => 0,
            Entrada::Rapidas(r) => {
                if r.is_empty() || y - self.y_de(i) > RAPIDO {
                    return None;
                }
                let paso = self.ancho_rapido(r.len()) + HUECO_RAPIDO;
                (((x - MARGEN) / paso).floor().max(0.0) as usize).min(r.len() - 1)
            }
            Entrada::Colores { .. } => {
                let n = crate::escritorio::MUESTRAS.len() + 1;
                self.centros_colores(n)
                    .iter()
                    .position(|c| (x - c).abs() <= ZONA_MUESTRA / 2.0)?
            }
            _ => return None,
        };
        self.objetivos
            .iter()
            .position(|o| *o == Objetivo { entrada: i, k })
    }

    fn ancho_rapido(&self, n: usize) -> f32 {
        (self.ancho - MARGEN * 2.0 - HUECO_RAPIDO * (n as f32 - 1.0)) / n as f32
    }

    /// Los centros de las muestras: repartidas a lo ancho del texto de las
    /// filas, con la primera y la última alineadas con el icono y el chevron.
    fn centros_colores(&self, n: usize) -> Vec<f32> {
        let (desde, hasta) = (
            MARGEN + RELLENO + ICONO / 2.0,
            self.ancho - MARGEN - RELLENO - ICONO / 2.0,
        );
        let paso = (hasta - desde) / (n as f32 - 1.0);
        (0..n).map(|k| desde + paso * k as f32).collect()
    }

    /// Lo que hace pulsar un objetivo, si hace algo.
    fn accion_de(&self, o: Objetivo) -> Option<Accion> {
        match &self.entradas[o.entrada] {
            Entrada::Fila(f) => f.accion.clone(),
            Entrada::Rapidas(r) => r.get(o.k).map(|b| b.accion.clone()),
            Entrada::Colores { elegido } => {
                Some(match crate::escritorio::MUESTRAS.get(o.k) {
                    // Pulsar el color que ya tienen lo quita, como en macOS:
                    // así no hace falta una muestra de «sin color».
                    Some(c) if elegido.as_deref() == Some(*c) => Accion::EscritorioColor(None),
                    Some(c) => Accion::EscritorioColor(Some((*c).to_string())),
                    None => Accion::EscritorioElegirColor,
                })
            }
            _ => None,
        }
    }

    fn tiene_sub(&self, o: Objetivo) -> bool {
        matches!(&self.entradas[o.entrada], Entrada::Fila(f) if !f.sub.is_empty())
    }

    // 0.1.0
    // modified 0.2.3 -- objetivos de grupo y submenús en cualquier fila
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((x, y)) = punto.map(|(x, y)| self.local(x, y)) else {
            return self.señalado.señalar(None) | self.señalada_sub.señalar(None);
        };
        if let Some(j) = self.fila_sub_en(x, y) {
            // Mientras se recorre el submenú su fila madre sigue encendida,
            // como en macOS: es lo que dice de dónde cuelga.
            let madre = self
                .abierto
                .and_then(|i| self.objetivos.iter().position(|o| o.entrada == i));
            self.foco_sub = false;
            return self.señalado.señalar(madre) | self.señalada_sub.señalar(Some(j));
        }
        let Some(n) = self.objetivo_en(x, y) else {
            // Fuera de todo objetivo no se cierra nada. Cruzar el borde del
            // menú hacia el submenú pasa por un píxel así, y cerrarlo ahí lo
            // volvería imposible de alcanzar.
            return false;
        };
        let o = self.objetivos[n];
        let abierto = self.tiene_sub(o).then_some(o.entrada);
        let cambia = self.abierto != abierto;
        self.abierto = abierto;
        self.foco_sub = false;
        self.señalado.señalar(Some(n)) | self.señalada_sub.señalar(None) | cambia
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalado.animando() || self.señalada_sub.animando()
    }

    // 0.1.0
    // modified 0.2.3 -- botones rápidos, colores y submenús
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (x, y) = self.local(x, y);
        if let Some(j) = self.fila_sub_en(x, y) {
            let i = self.abierto?;
            return self.filas_sub(i)[j].1.accion.clone();
        }
        let o = self.objetivos[self.objetivo_en(x, y)?];
        if self.tiene_sub(o) {
            // Para quien no usa el ratón con hover (táctil): el toque abre el
            // submenú, y el siguiente elige.
            self.abierto = Some(o.entrada);
            self.foco_sub = false;
            self.señalada_sub.señalar(None);
            return None;
        }
        self.accion_de(o)
    }

    // 0.1.0
    // modified 0.2.3 -- Izquierda/Derecha recorren los grupos horizontales
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        if self.foco_sub
            && let Some(i) = self.abierto
        {
            let n = self.filas_sub(i).len();
            let actual = self.señalada_sub.actual();
            match tecla {
                T::Arriba => {
                    self.señalada_sub
                        .señalar(Some(actual.map_or(n - 1, |a| (a + n - 1) % n)));
                    return Tecla::Consumida;
                }
                T::Abajo | T::Tabulador => {
                    self.señalada_sub
                        .señalar(Some(actual.map_or(0, |a| (a + 1) % n)));
                    return Tecla::Consumida;
                }
                T::Intro => {
                    return match actual.and_then(|j| self.filas_sub(i)[j].1.accion.clone()) {
                        Some(accion) => Tecla::Hacer(accion),
                        None => Tecla::Consumida,
                    };
                }
                T::Izquierda | T::Escape => {
                    self.foco_sub = false;
                    self.abierto = None;
                    self.señalada_sub.señalar(None);
                    return Tecla::Consumida;
                }
                _ => {}
            }
        }
        if self.objetivos.is_empty() {
            return match tecla {
                T::Escape => Tecla::Cerrar,
                _ => Tecla::Ignorada,
            };
        }
        let n = self.objetivos.len();
        let actual = self.señalado.actual();
        let objetivo = actual.map(|a| self.objetivos[a]);
        // En una fila, Izquierda/Derecha no son «el de al lado»: Derecha abre
        // su submenú y la Izquierda no hace nada. Dentro de un grupo
        // horizontal, recorren el grupo.
        let en_grupo = objetivo.is_some_and(|o| {
            matches!(
                self.entradas[o.entrada],
                Entrada::Rapidas(_) | Entrada::Colores { .. }
            )
        });
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Derecha | T::Intro if objetivo.is_some_and(|o| self.tiene_sub(o)) => {
                self.abierto = objetivo.map(|o| o.entrada);
                self.foco_sub = true;
                self.señalada_sub.señalar(Some(0));
                Tecla::Consumida
            }
            T::Izquierda | T::Derecha if en_grupo => {
                let a = actual.expect("en un grupo hay objetivo señalado");
                let entrada = self.objetivos[a].entrada;
                let b = if tecla == T::Derecha {
                    a + 1
                } else {
                    a.wrapping_sub(1)
                };
                if self.objetivos.get(b).is_some_and(|o| o.entrada == entrada) {
                    self.señalado.señalar(Some(b));
                }
                Tecla::Consumida
            }
            // Arriba y Abajo saltan de **entrada** en entrada: un grupo
            // horizontal es una parada, no siete.
            T::Arriba | T::Abajo | T::Tabulador => {
                let paradas: Vec<usize> = (0..n)
                    .filter(|&m| {
                        m == 0 || self.objetivos[m].entrada != self.objetivos[m - 1].entrada
                    })
                    .collect();
                let aqui = actual.and_then(|a| paradas.iter().rposition(|&p| p <= a));
                let siguiente = match (tecla, aqui) {
                    (T::Arriba, None) => paradas.len() - 1,
                    (T::Arriba, Some(p)) => (p + paradas.len() - 1) % paradas.len(),
                    (_, None) => 0,
                    (_, Some(p)) => (p + 1) % paradas.len(),
                };
                self.señalado.señalar(Some(paradas[siguiente]));
                // Moverse a otra fila cierra el submenú que hubiera.
                self.abierto = None;
                self.foco_sub = false;
                self.señalada_sub.señalar(None);
                Tecla::Consumida
            }
            T::Intro => match objetivo.and_then(|o| self.accion_de(o)) {
                Some(accion) => Tecla::Hacer(accion),
                None => Tecla::Consumida,
            },
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    // modified 0.2.3 -- cabecera, botones rápidos, colores, títulos y marcas
    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(Length::Fixed(BORDE))];
        for (i, entrada) in self.entradas.iter().enumerate() {
            col = col.push(self.ver_entrada(i, entrada));
        }
        col = col.push(Space::new().height(Length::Fixed(BORDE)));
        let principal = super::menu::caja(col.into(), self.ancho);

        let mut lienzo = row![];
        if self.origen.0 > 0.0 {
            lienzo = lienzo.push(Space::new().width(Length::Fixed(self.origen.0)));
        }
        lienzo = lienzo.push(
            column![Space::new().height(Length::Fixed(self.origen.1)), principal]
                .width(Length::Fixed(self.ancho)),
        );
        if let Some(i) = self.abierto {
            let mut filas = column![Space::new().height(Length::Fixed(BORDE))];
            let mut j = 0;
            for e in self.sub_de(i) {
                filas = filas.push(match e {
                    Entrada::Fila(f) => {
                        let señalada = self.señalada_sub.intensidad(j);
                        j += 1;
                        fila_con(
                            &f.etiqueta,
                            f.atajo,
                            f.delante(),
                            f.peligro,
                            false,
                            señalada,
                        )
                    }
                    Entrada::Divisor => divisor(),
                    Entrada::Titulo(t) => titulo(t),
                    // Un submenú es una lista: lo demás no tiene sentido dentro.
                    _ => Space::new().into(),
                });
            }
            filas = filas.push(Space::new().height(Length::Fixed(BORDE)));
            let (y, _) = self.caja_sub(i);
            lienzo = lienzo.push(Space::new().width(Length::Fixed(HUECO_SUBMENU)));
            lienzo = lienzo.push(column![
                Space::new().height(Length::Fixed(self.origen.1 + y)),
                super::menu::caja(filas.into(), self.ancho_sub)
            ]);
        }
        lienzo.into()
    }

    fn intensidad(&self, entrada: usize, k: usize) -> f32 {
        self.objetivos
            .iter()
            .position(|o| *o == Objetivo { entrada, k })
            .map_or(0.0, |n| self.señalado.intensidad(n))
    }

    fn ver_entrada<'a>(&'a self, i: usize, entrada: &'a Entrada) -> PanelElement<'a> {
        match entrada {
            Entrada::Fila(f) if self.ventana => {
                let realce = self.intensidad(i, 0);
                container(
                    text(super::recortar_texto(&f.etiqueta, 270.0, 14.0))
                        .size(14)
                        .color(tema::texto()),
                )
                .padding([14, 20])
                .width(self.ancho)
                .height(self.alto_fila())
                .style(move |_| container::Style {
                    background: Some(tema::alfa(tema::acento(), realce * 0.10).into()),
                    border: Border {
                        radius: tema::R_ITEM_POPOVER.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
            }
            Entrada::Fila(f) => fila_con(
                &f.etiqueta,
                f.atajo,
                f.delante(),
                f.peligro,
                !f.sub.is_empty(),
                self.intensidad(i, 0),
            ),
            Entrada::Divisor => divisor(),
            Entrada::Titulo(t) => titulo(t),
            Entrada::Cabecera {
                icono,
                nombre,
                detalle,
            } => self.cabecera(icono.as_ref(), nombre, detalle),
            Entrada::Rapidas(botones) => self.rapidas(i, botones),
            Entrada::Colores { elegido } => self.colores(i, elegido.as_deref()),
        }
    }

    fn cabecera<'a>(
        &self,
        icono: Option<&'a Icono>,
        nombre: &str,
        detalle: &str,
    ) -> PanelElement<'a> {
        // El texto empieza donde lo deja la miniatura; lo que no quepa se
        // recorta con puntos, nunca se parte: la cabecera tiene un alto fijo.
        let sitio = self.ancho - MARGEN * 2.0 - MINI - HUECO_ICONO;
        let mini: PanelElement<'a> = match icono {
            Some(ic) => crate::icono::ver(ic, MINI, MINI),
            None => Space::new().width(Length::Fixed(MINI)).into(),
        };
        container(
            row![
                mini,
                Space::new().width(Length::Fixed(HUECO_ICONO)),
                column![
                    text(super::recortar_texto(nombre, sitio, tema::T_CUERPO))
                        .size(tema::T_CUERPO)
                        .font(Font {
                            weight: Weight::Semibold,
                            ..Font::DEFAULT
                        })
                        .color(tema::texto()),
                    text(super::recortar_texto(detalle, sitio, 11.0))
                        .size(11)
                        .color(tema::texto2()),
                ]
                .spacing(2),
            ]
            .align_y(iced_core::alignment::Vertical::Center),
        )
        // 5 arriba como el margen de las filas, y 9 abajo hasta los botones.
        .padding(iced_core::Padding {
            top: 5.0,
            bottom: 9.0,
            left: MARGEN,
            right: MARGEN,
        })
        .height(Length::Fixed(CABECERA))
        .into()
    }

    fn rapidas(&self, i: usize, botones: &[Rapida]) -> PanelElement<'_> {
        let ancho = self.ancho_rapido(botones.len());
        let mut fila = row![].spacing(HUECO_RAPIDO);
        for (k, b) in botones.iter().enumerate() {
            let señalada = self.intensidad(i, k);
            let sobre = tema::sobre_acento();
            let color = tema::mezclar(tema::texto(), sobre, señalada);
            let fondo = tema::alfa(tema::acento(), señalada);
            let icono: PanelElement<'_> = match crate::icono::propio(b.icono) {
                Some(ic) => crate::icono::ver_teñido_propio(&ic, 20.0, color),
                None => Space::new().height(Length::Fixed(20.0)).into(),
            };
            fila = fila.push(
                container(
                    column![
                        icono,
                        text(super::recortar_texto(&b.etiqueta, ancho - 8.0, 11.0))
                            .size(11)
                            .font(Font {
                                weight: Weight::Medium,
                                ..Font::DEFAULT
                            })
                            .color(color)
                    ]
                    .spacing(4)
                    .align_x(iced_core::alignment::Horizontal::Center),
                )
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(RAPIDO))
                .center_x(Length::Fixed(ancho))
                .center_y(Length::Fixed(RAPIDO))
                .style(move |_| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: tema::R_ITEM_POPOVER.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        column![
            container(fila).padding([0, MARGEN as u16]),
            Space::new().height(Length::Fixed(5.0)),
            container(
                container(Space::new().height(Length::Fixed(1.0)))
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        background: Some(tema::divisor().into()),
                        ..Default::default()
                    })
            )
            .padding([0, (MARGEN + RELLENO) as u16]),
            Space::new().height(Length::Fixed(5.0)),
        ]
        .into()
    }

    fn colores(&self, i: usize, elegido: Option<&str>) -> PanelElement<'_> {
        let muestras = crate::escritorio::MUESTRAS;
        let n = muestras.len() + 1;
        let centros = self.centros_colores(n);
        let mut fila = row![].align_y(iced_core::alignment::Vertical::Center);
        let mut x = 0.0;
        for (k, centro) in centros.iter().enumerate() {
            let izq = centro - ZONA_MUESTRA / 2.0;
            fila = fila.push(Space::new().width(Length::Fixed(izq - x)));
            x = izq + ZONA_MUESTRA;
            let señalada = self.intensidad(i, k);
            let pieza: PanelElement<'_> = match muestras.get(k) {
                Some(hex) => {
                    let c = tema::color_css(hex).unwrap_or(Color::BLACK);
                    // La elegida lleva un anillo de **su** color, no azul: el
                    // azul ya dice «acción», y aquí lo que se marca es un
                    // valor. El hover pinta el mismo anillo más tenue.
                    let anillo = if elegido == Some(*hex) {
                        1.0
                    } else {
                        0.45 * señalada
                    };
                    muestra(c, anillo)
                }
                None => mas(señalada),
            };
            fila = fila.push(
                container(pieza)
                    .width(Length::Fixed(ZONA_MUESTRA))
                    .height(Length::Fixed(ZONA_MUESTRA))
                    .center_x(Length::Fixed(ZONA_MUESTRA))
                    .center_y(Length::Fixed(ZONA_MUESTRA)),
            );
        }
        container(fila)
            .height(Length::Fixed(COLORES))
            .center_y(Length::Fixed(COLORES))
            .into()
    }
}

/// Hueco entre dos botones rápidos.
const HUECO_RAPIDO: f32 = 2.0;

fn titulo(t: &str) -> PanelElement<'static> {
    container(
        text(t.to_uppercase())
            .size(tema::T_PEQUENO - 1.0)
            .font(Font {
                weight: Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(tema::texto2()),
    )
    .padding(iced_core::Padding {
        top: 7.0,
        bottom: 4.0,
        left: MARGEN + RELLENO,
        right: MARGEN + RELLENO,
    })
    .height(Length::Fixed(TITULO))
    .into()
}

/// Una muestra de color con su anillo: 2 de separación del color de la
/// tarjeta y 1,5 de anillo, como el contorno de elegida del HIG §9.
fn muestra(c: Color, anillo: f32) -> PanelElement<'static> {
    let lado = MUESTRA + 2.0 * 2.0 + 1.5 * 2.0;
    container(
        container(Space::new())
            .width(Length::Fixed(MUESTRA))
            .height(Length::Fixed(MUESTRA))
            .style(move |_| container::Style {
                background: Some(c.into()),
                border: Border {
                    radius: (MUESTRA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .width(Length::Fixed(lado))
    .height(Length::Fixed(lado))
    .center_x(Length::Fixed(lado))
    .center_y(Length::Fixed(lado))
    .style(move |_| container::Style {
        border: Border {
            radius: (lado / 2.0).into(),
            width: 1.5,
            color: tema::alfa(c, anillo),
        },
        ..Default::default()
    })
    .into()
}

/// El «+» del color personalizado: abre el selector de siempre.
fn mas(señalada: f32) -> PanelElement<'static> {
    let lado = 20.0;
    let fondo = tema::mezclar(
        tema::alfa(tema::tinta(), 0.08),
        tema::alfa(tema::tinta(), 0.16),
        señalada,
    );
    let color = tema::mezclar(tema::texto2(), tema::texto(), señalada);
    container(match crate::icono::propio("mas") {
        Some(ic) => crate::icono::ver_teñido_propio(&ic, 12.0, color),
        None => Space::new().into(),
    })
    .width(Length::Fixed(lado))
    .height(Length::Fixed(lado))
    .center_x(Length::Fixed(lado))
    .center_y(Length::Fixed(lado))
    .style(move |_| container::Style {
        background: Some(fondo.into()),
        border: Border {
            radius: (lado / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escritorio::NuevoElemento;

    #[test]
    fn teclado_selecciona_el_destino_y_escape_cancela() {
        let mut menu = MenuDock::ventana(vec![
            ("Siempre encima".into(), Accion::VentanaEncima),
            ("Mover a Escritorio 2".into(), Accion::VentanaEscritorio(1)),
        ]);
        menu.tecla(crate::TeclaPulsada::Abajo);
        menu.tecla(crate::TeclaPulsada::Abajo);
        assert!(matches!(
            menu.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::VentanaEscritorio(1))
        ));
        assert!(matches!(
            menu.tecla(crate::TeclaPulsada::Escape),
            Tecla::Cerrar
        ));
    }

    fn crear() -> Vec<Entrada> {
        [
            ("Carpeta", NuevoElemento::Carpeta),
            ("Documento de texto", NuevoElemento::Texto),
        ]
        .into_iter()
        .map(|(e, t)| Entrada::Fila(Fila::new(e, "carpeta-nueva", Accion::EscritorioNuevo(t))))
        .collect()
    }

    fn con_crear() -> MenuDock {
        MenuDock::escritorio(
            vec![
                Entrada::Fila(Fila::submenu("Crear", "carpeta-nueva", crear())),
                Entrada::Divisor,
                Entrada::Fila(Fila::new("Cambiar fondo…", "fondo", Accion::AbrirCaptura)),
            ],
            (0.0, 0.0),
        )
    }

    /// Pasar sobre «Crear» abre su lista a la derecha; sobre otra fila, la
    /// cierra; y cruzar el hueco entre las dos no la cierra.
    #[test]
    fn el_submenu_se_abre_al_pasar_y_se_cierra_al_salir() {
        let mut m = con_crear();
        let y = BORDE + FILA / 2.0;
        assert!(m.size().0 > m.ancho, "reserva el sitio del submenú");
        m.puntero(Some((20.0, y)));
        assert_eq!(m.abierto, Some(0));
        m.puntero(Some((m.ancho + 1.0, y)));
        assert_eq!(m.abierto, Some(0), "el hueco no lo cierra");
        m.puntero(Some((20.0, BORDE + FILA + DIVISOR + FILA / 2.0)));
        assert_eq!(m.abierto, None);
    }

    /// El tamaño no cambia al abrir un submenú: si cambiara, el compositor
    /// rehará la superficie y la entrada volvería a empezar.
    #[test]
    fn abrir_el_submenu_no_cambia_el_tamano() {
        let mut m = con_crear();
        let antes = m.size();
        m.puntero(Some((20.0, BORDE + FILA / 2.0)));
        assert_eq!(m.size(), antes);
    }

    #[test]
    fn las_filas_del_submenu_hacen_lo_suyo() {
        let mut m = con_crear();
        m.puntero(Some((20.0, BORDE + FILA / 2.0)));
        let x = m.ancho + HUECO_SUBMENU + 20.0;
        let y = |j: f32| BORDE + BORDE + FILA * j + FILA / 2.0;
        assert_eq!(
            m.pulsar(x, y(1.0)),
            Some(Accion::EscritorioNuevo(NuevoElemento::Texto))
        );
    }

    #[test]
    fn el_submenu_se_maneja_con_teclado() {
        use crate::TeclaPulsada as T;
        let mut m = con_crear();
        m.tecla(T::Abajo);
        m.tecla(T::Derecha);
        m.tecla(T::Abajo);
        assert!(matches!(
            m.tecla(T::Intro),
            Tecla::Hacer(Accion::EscritorioNuevo(NuevoElemento::Texto))
        ));
        assert!(matches!(m.tecla(T::Izquierda), Tecla::Consumida));
        assert!(matches!(m.tecla(T::Escape), Tecla::Cerrar));
    }

    fn carpeta() -> MenuDock {
        MenuDock::escritorio(
            vec![
                Entrada::Rapidas(vec![
                    Rapida {
                        etiqueta: "Abrir".into(),
                        icono: "abrir",
                        accion: Accion::Lanzar("a".into()),
                    },
                    Rapida {
                        etiqueta: "Copiar".into(),
                        icono: "copiar",
                        accion: Accion::Lanzar("c".into()),
                    },
                ]),
                Entrada::Colores {
                    elegido: Some(crate::escritorio::MUESTRAS[1].into()),
                },
                Entrada::Divisor,
                Entrada::Fila(
                    Fila::new(
                        "Mover a la papelera",
                        "papelera",
                        Accion::EscritorioEliminarSeleccion,
                    )
                    .peligro(),
                ),
            ],
            (0.0, 0.0),
        )
    }

    /// Cada botón rápido hace lo suyo, y pulsar el color que ya tienen lo
    /// quita en vez de volver a ponerlo.
    #[test]
    fn botones_y_colores_se_pulsan() {
        let mut m = carpeta();
        let ancho = m.ancho;
        assert_eq!(
            m.pulsar(ancho - 20.0, BORDE + 10.0),
            Some(Accion::Lanzar("c".into()))
        );
        let y = BORDE + RAPIDAS + COLORES / 2.0;
        let centros = m.centros_colores(crate::escritorio::MUESTRAS.len() + 1);
        assert_eq!(m.pulsar(centros[1], y), Some(Accion::EscritorioColor(None)));
        assert_eq!(
            m.pulsar(centros[0], y),
            Some(Accion::EscritorioColor(Some(
                crate::escritorio::MUESTRAS[0].into()
            )))
        );
        assert_eq!(
            m.pulsar(*centros.last().expect("hay muestras"), y),
            Some(Accion::EscritorioElegirColor)
        );
    }

    /// Abajo salta de entrada en entrada; un grupo cuenta como una parada y
    /// Derecha recorre sus botones.
    #[test]
    fn el_teclado_recorre_los_grupos() {
        use crate::TeclaPulsada as T;
        let mut m = carpeta();
        m.tecla(T::Abajo);
        m.tecla(T::Derecha);
        assert!(matches!(m.tecla(T::Intro), Tecla::Hacer(Accion::Lanzar(c)) if c == "c"));
        m.tecla(T::Abajo);
        m.tecla(T::Abajo);
        assert!(matches!(
            m.tecla(T::Intro),
            Tecla::Hacer(Accion::EscritorioEliminarSeleccion)
        ));
    }

    /// En el dock el menú queda centrado: lo que se reserva para el submenú
    /// va a los dos lados, y el submenú no baja por debajo del menú.
    #[test]
    fn el_dock_reserva_a_los_dos_lados() {
        let m = MenuDock::dock(
            vec![
                Entrada::Fila(Fila::new(
                    "Nueva ventana",
                    "ventana",
                    Accion::Lanzar("x".into()),
                )),
                Entrada::Fila(Fila::submenu(
                    "Opciones",
                    "preferencias",
                    vec![Entrada::Fila(Fila::marcada(
                        "Mantener en el dock",
                        Marca::Casilla(true),
                        Accion::Lanzar("y".into()),
                    ))],
                )),
            ],
            100.0,
        );
        let (w, h) = m.size();
        assert!((m.origen.0 * 2.0 + m.ancho - w).abs() < 0.01);
        assert!(
            (h - m.alto_principal()).abs() < 0.01,
            "el submenú cabe hacia arriba"
        );
    }

    #[test]
    fn el_menu_de_fondo_del_escritorio_tiene_cabecera_rapidas_y_ancho_rico() {
        for visible in [true, false] {
            let esc = crate::escritorio::Escritorio::new(32.0, (1920.0, 1080.0), visible, crate::escritorio::Tamano::Mediano);
            let entradas = crate::menus::escritorio(&esc, false);
            assert!(matches!(entradas.first(), Some(Entrada::Cabecera { .. })));
            assert!(matches!(
                entradas.get(1),
                Some(Entrada::Rapidas(botones)) if botones.len() == 3
            ));
            let m = MenuDock::escritorio(entradas, (100.0, 100.0));
            assert!(m.ancho >= ANCHO_RICO, "ancho={} < {ANCHO_RICO}", m.ancho);
        }
    }

    #[test]
    fn volver_al_menu_principal_con_puntero_limpia_el_foco_del_submenu() {
        use crate::TeclaPulsada as T;
        let mut m = MenuDock::dock(
            vec![
                Entrada::Fila(Fila::new(
                    "Nueva ventana",
                    "ventana",
                    Accion::Lanzar("uno".into()),
                )),
                Entrada::Fila(Fila::submenu(
                    "Opciones",
                    "preferencias",
                    vec![Entrada::Fila(Fila::marcada(
                        "Mantener",
                        Marca::Casilla(true),
                        Accion::Lanzar("sub".into()),
                    ))],
                )),
            ],
            100.0,
        );
        m.tecla(T::Abajo);
        m.tecla(T::Abajo);
        m.tecla(T::Derecha);
        assert!(m.foco_sub);
        assert_eq!(m.abierto, Some(1));

        let x = m.origen.0 + m.ancho / 2.0;
        assert!(m.puntero(Some((x, m.origen.1 + m.y_de(0) + FILA / 2.0))));
        assert!(!m.foco_sub);
        assert_eq!(m.abierto, None);

        assert!(m.puntero(Some((x, m.origen.1 + m.y_de(1) + FILA / 2.0))));
        assert_eq!(m.abierto, Some(1));
        assert!(!m.foco_sub);
    }
}
