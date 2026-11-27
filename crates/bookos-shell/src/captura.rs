//! La capa de captura de pantalla: elegir qué se captura.
//!
//! Tapa la pantalla entera, como el bloqueo, y por el mismo motivo: tiene que
//! taparlo todo y quedarse con el ratón y el teclado mientras dura. Se abre con Impr y se va con Escape.
//!
//! ## El velo son cuatro bandas, y no las pinta el shell
//!
//! Lo que se recorta se ve **sin velo** y el resto atenuado, que es como se
//! entiende de un vistazo qué va a salir en la foto. El velo son cuatro
//! rectángulos alrededor de la selección —arriba, abajo, izquierda y derecha—
//! y el recuadro otros cuatro, y **los compone el compositor** con la GPU a
//! partir de [`Captura::marcado`].
//!
//! Antes eran contenedores de iced dentro de un lienzo a pantalla completa, y
//! eso hacía que arrastrar fuera a tirones: medido en release a 2880×1800 y
//! escala 1,75, cada movimiento del ratón costaba **24,5 ms** de rasterizado en
//! CPU, y se pagaba por evento, no por fotograma. El coste de `tiny-skia` va con
//! el tamaño del buffer —ver `Shell::paint`—, así que lo que queda aquí son dos
//! lienzos pequeños: la barra de modos, que casi nunca cambia, y la pastilla de
//! las medidas, que cambia con cada píxel pero mide 132×26.
//!
//! ## Marcar no es capturar
//!
//! Soltar el arrastre deja el recuadro marcado y ahí se queda: la foto la hacen
//! los botones «Copiar» y «Guardar» de la barra, o Intro, que copia. Así se
//! decide qué hacer con la captura **viéndola ya encuadrada**, y se puede
//! rehacer el recuadro las veces que haga falta antes de elegir.
//!
//! ## Lo marcado se ajusta
//!
//! Con un recuadro ya marcado, pulsar cerca de un borde o de una esquina tira
//! de ese lado, y pulsar dentro lo mueve entero. Solo fuera empieza uno nuevo.
//! Sin esto, corregir cinco píxeles de alto obligaba a repetir el arrastre
//! entero y acertar otra vez las otras tres aristas.
//!
//! El recuadro lleva las esquinas redondeadas, con el radio de un control del
//! sistema: las redondea el shader del velo en el compositor.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use image::ImageEncoder as _;

use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

/// Grosor del recuadro de la selección, en lógicos.
pub const BORDE: f32 = 2.0;
/// Lo oscuro que se pone lo que **no** entra en la captura.
///
/// 0,45 y no el 0,82 del launchpad: aquí lo de debajo hay que **verlo** para
/// poder encuadrar. Es el mismo velo que el buscador.
pub const VELO: f32 = 0.45;
/// Alto de la pastilla con las medidas.
pub const PASTILLA_ALTO: f32 = 26.0;
pub const PASTILLA_ANCHO: f32 = 132.0;
/// Aire entre la selección y la pastilla.
const PASTILLA_HUECO: f32 = 8.0;

/// Dimensiones 1:1 de la barra flotante inferior (concéntrica: 20 - 6 = 14, 14 - 3 = 11).
const BARRA_ALTO: f32 = 48.0;
const BARRA_MARGEN: f32 = 22.0;
const BARRA_PADDING: f32 = 6.0;
const BARRA_RADIO: f32 = 20.0;
const CONTROL_ALTO: f32 = 36.0;
const CONTROL_RADIO: f32 = 14.0;
const SEG_PADDING: f32 = 3.0;
const THUMB_ALTO: f32 = 30.0;
const THUMB_RADIO: f32 = 11.0;
const GAP_BARRA: f32 = 6.0;
const GAP_SEG: f32 = 2.0;

const CERRAR_W: f32 = 36.0;
const TIPO_BTN_W: f32 = 76.0;
const AREA_BTN_W: f32 = 96.0;
const OPCIONES_W: f32 = 104.0;
const ACCION_W: f32 = 104.0;

const DIVISOR_ANCHO: f32 = 1.0;

/// Dimensiones del popover de opciones sobre el botón «Opciones».
pub const OPCIONES_POPOVER_W: f32 = 304.0;
pub const OPCIONES_POPOVER_H_FOTO: f32 = 180.0;
pub const OPCIONES_POPOVER_H_VIDEO: f32 = 180.0;
const OPCIONES_POPOVER_HUECO: f32 = 10.0;

/// Dimensiones del pill flotante de grabación (excluido del vídeo por el compositor).
pub const PILL_GRABACION_W: f32 = 252.0;
pub const PILL_GRABACION_H: f32 = 42.0;

/// Lo mínimo que tiene que medir una selección para valer.
///
/// Por debajo de esto es un clic con la mano temblando, no un recuadro: sin el
/// umbral, pulsar para cancelar acababa guardando una captura de tres píxeles.
const MINIMO: f32 = 8.0;

/// Hasta dónde, a cada lado de una arista, se la puede agarrar. 8 lógicos son
/// 14 físicos a escala 1,75: menos que eso cuesta acertar con el touchpad, y
/// más se come el interior de recuadros pequeños, que es donde se mueve.
const AGARRE: f32 = 8.0;

/// Qué arista mueve un agarre en un eje.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lado {
    Ninguno,
    /// La de arriba o la de la izquierda.
    Inicio,
    /// La de abajo o la de la derecha.
    Fin,
}

/// De dónde se ha cogido un recuadro ya marcado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Agarre {
    /// Por dentro: se mueve entero sin cambiar de tamaño.
    Mover,
    /// Por un borde o una esquina: cada eje dice qué arista estira.
    Estirar { x: Lado, y: Lado },
}

/// Qué forma pide el cursor según lo que haría un clic ahí. El shell no sabe
/// de temas de cursor: esto lo traduce el compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forma {
    /// Marcar un recuadro nuevo.
    Cruz,
    Mover,
    Horizontal,
    Vertical,
    /// Esquina de arriba a la izquierda o de abajo a la derecha.
    DiagonalBajando,
    /// Esquina de arriba a la derecha o de abajo a la izquierda.
    DiagonalSubiendo,
    /// La barra de botones.
    Flecha,
}

/// El arrastre en curso.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Arrastre {
    /// Un recuadro nuevo: dónde empezó y por dónde va.
    Nuevo((f32, f32), (f32, f32)),
    /// Ajustando uno marcado: cómo era al cogerlo, por dónde se cogió, y el
    /// punto de partida y el actual del ratón.
    Ajuste {
        antes: Recuadro,
        agarre: Agarre,
        desde: (f32, f32),
        hasta: (f32, f32),
    },
}

/// A dónde va la captura. Son los dos botones que la **hacen**: marcar solo
/// encuadra, y lo que pase con la foto se decide después de verla marcada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destino {
    /// Al portapapeles, para pegarla donde sea. Es lo que hace Intro: la
    /// mayoría de las capturas se pegan en un chat y no se vuelven a mirar.
    Portapapeles,
    /// A un fichero en la carpeta de capturas.
    Guardar,
}

impl Destino {
    const TODOS: [Self; 2] = [Self::Portapapeles, Self::Guardar];
}

/// Tipo de captura: foto o vídeo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoCaptura {
    Foto,
    Video,
}

impl TipoCaptura {
    const TODOS: [Self; 2] = [Self::Foto, Self::Video];

    fn nombre(self) -> &'static str {
        match self {
            Self::Foto => crate::emergente::t("Foto", "Photo"),
            Self::Video => crate::emergente::t("Vídeo", "Video"),
        }
    }

    fn simbolo(self) -> &'static str {
        match self {
            Self::Foto => "foto",
            Self::Video => "video",
        }
    }
}

/// Fuente de audio al grabar la pantalla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioGrabacion {
    Ninguno,
    Mic,
    Sistema,
    Ambos,
}

impl AudioGrabacion {
    const TODOS: [Self; 4] = [Self::Ninguno, Self::Mic, Self::Sistema, Self::Ambos];

    fn nombre(self) -> &'static str {
        match self {
            Self::Ninguno => crate::emergente::t("Sin audio", "Muted"),
            Self::Mic => crate::emergente::t("Micrófono", "Mic"),
            Self::Sistema => crate::emergente::t("Sistema", "System"),
            Self::Ambos => crate::emergente::t("Ambos", "Both"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// La pantalla entera, sin recortar.
    Pantalla,
    /// Una ventana elegida con el puntero.
    Ventana,
    /// Lo que se marque arrastrando.
    Seleccion,
}

impl Modo {
    const TODOS: [Self; 3] = [Self::Pantalla, Self::Ventana, Self::Seleccion];

    // 0.1.0
    fn nombre(self) -> &'static str {
        match self {
            Self::Pantalla => crate::emergente::t("Pantalla", "Screen"),
            Self::Ventana => crate::emergente::t("Ventana", "Window"),
            Self::Seleccion => crate::emergente::t("Selección", "Selection"),
        }
    }

    /// El icono propio que lo dibuja.
    // 0.1.0
    fn simbolo(self) -> &'static str {
        match self {
            Self::Pantalla => "monitor",
            Self::Ventana => "ventana",
            Self::Seleccion => "seleccion",
        }
    }
}

/// Un rectángulo en lógicos, con el origen arriba a la izquierda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recuadro {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Recuadro {
    /// El rectángulo entre dos puntos, en cualquier orden: se arrastra hacia
    /// donde se quiera y el recuadro sale siempre con lados positivos.
    // 0.1.0
    fn entre(a: (f32, f32), b: (f32, f32)) -> Self {
        Self {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (b.0 - a.0).abs(),
            h: (b.1 - a.1).abs(),
        }
    }

    // 0.1.0
    fn vale(&self) -> bool {
        self.w >= MINIMO && self.h >= MINIMO
    }

    /// Por dónde se coge el recuadro en ese punto, o `None` si queda fuera.
    ///
    /// Los bordes ganan al interior hasta [`AGARRE`] hacia dentro: si no, en un
    /// recuadro estrecho no habría forma de coger la arista.
    // 0.1.0
    fn agarre(&self, x: f32, y: f32) -> Option<Agarre> {
        let dentro_x = x >= self.x - AGARRE && x <= self.x + self.w + AGARRE;
        let dentro_y = y >= self.y - AGARRE && y <= self.y + self.h + AGARRE;
        if !(dentro_x && dentro_y) {
            return None;
        }
        let lado = |p: f32, ini: f32, fin: f32| {
            if (p - ini).abs() <= AGARRE {
                Lado::Inicio
            } else if (p - fin).abs() <= AGARRE {
                Lado::Fin
            } else {
                Lado::Ninguno
            }
        };
        match (
            lado(x, self.x, self.x + self.w),
            lado(y, self.y, self.y + self.h),
        ) {
            (Lado::Ninguno, Lado::Ninguno) => Some(Agarre::Mover),
            (x, y) => Some(Agarre::Estirar { x, y }),
        }
    }

    /// El recuadro tras arrastrar `d` desde el agarre, dentro de la pantalla.
    // 0.1.0
    fn ajustado(&self, agarre: Agarre, d: (f32, f32), pantalla: (f32, f32)) -> Self {
        match agarre {
            // Se para en el borde en vez de encogerse: moverlo no cambia qué
            // tamaño se eligió.
            Agarre::Mover => Self {
                x: (self.x + d.0).clamp(0.0, (pantalla.0 - self.w).max(0.0)),
                y: (self.y + d.1).clamp(0.0, (pantalla.1 - self.h).max(0.0)),
                ..*self
            },
            Agarre::Estirar { x, y } => {
                let tira = |lado, ini: f32, fin: f32, d: f32, max: f32| match lado {
                    Lado::Inicio => ((ini + d).clamp(0.0, max), fin),
                    Lado::Fin => (ini, (fin + d).clamp(0.0, max)),
                    Lado::Ninguno => (ini, fin),
                };
                let (x0, x1) = tira(x, self.x, self.x + self.w, d.0, pantalla.0);
                let (y0, y1) = tira(y, self.y, self.y + self.h, d.1, pantalla.1);
                // `entre` da la vuelta si una arista cruza la de enfrente, que
                // es lo que se espera al pasarse tirando.
                Self::entre((x0, y0), (x1, y1))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CeldaBarra {
    Cerrar,
    Tipo(usize),
    Area(usize),
    Opciones,
    Accion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CeldaPopover {
    Retardo(usize),
    Cursor,
    Destino(usize),
    Audio(usize),
}

pub struct Captura {
    /// La pantalla, en lógicos.
    pantalla: (f32, f32),
    tipo: TipoCaptura,
    modo: Modo,
    destino: Destino,
    audio: AudioGrabacion,
    arrastre: Option<Arrastre>,
    /// Lo último que se marcó y sigue marcado tras soltar.
    hecho: Option<Recuadro>,
    /// Ventana bajo el puntero cuando está activo `Modo::Ventana`.
    ventana_hover: Option<Recuadro>,
    /// Qué control de la barra tiene el ratón encima.
    señalada: Option<CeldaBarra>,
    /// Qué control del popover de opciones tiene el ratón encima.
    señalada_popover: Option<CeldaPopover>,
    popover_abierto: bool,
    /// Si el cursor sale en la foto o el vídeo.
    cursor: bool,
    /// Segundos de espera antes de capturar o empezar a grabar: 0 es inmediata.
    retardo: u8,
}

impl Captura {
    // 0.1.0
    pub fn new(pantalla: (f32, f32)) -> Self {
        let pantalla = (
            if pantalla.0.is_finite() { pantalla.0.max(1.0) } else { 1.0 },
            if pantalla.1.is_finite() { pantalla.1.max(1.0) } else { 1.0 },
        );
        Self {
            pantalla,
            tipo: TipoCaptura::Foto,
            modo: Modo::Seleccion,
            destino: Destino::Portapapeles,
            audio: AudioGrabacion::Mic,
            arrastre: None,
            hecho: None,
            ventana_hover: None,
            señalada: None,
            señalada_popover: None,
            popover_abierto: false,
            cursor: false,
            retardo: 0,
        }
    }

    /// Con las opciones que se dejaron la última vez.
    // añadido en la 0.2.0
    pub fn con_opciones(mut self, cursor: bool, retardo: u8) -> Self {
        self.cursor = cursor;
        self.retardo = retardo;
        self
    }

    // 0.1.0
    pub fn modo(&self) -> Modo {
        self.modo
    }

    pub fn tipo(&self) -> TipoCaptura {
        self.tipo
    }

    pub fn popover_abierto(&self) -> bool {
        self.popover_abierto
    }

    /// Actualiza el rectángulo de la ventana bajo el puntero en `Modo::Ventana`.
    pub fn establecer_ventana(&mut self, rect: Option<Recuadro>) -> bool {
        if self.ventana_hover == rect {
            return false;
        }
        self.ventana_hover = rect;
        self.modo == Modo::Ventana
    }

    /// Lo que se capturaría ahora mismo. `None` cuando en modo selección
    /// todavía no se ha marcado nada que valga.
    // 0.1.0
    pub fn recuadro(&self) -> Option<Recuadro> {
        match self.modo {
            Modo::Pantalla => Some(Recuadro {
                x: 0.0,
                y: 0.0,
                w: self.pantalla.0,
                h: self.pantalla.1,
            }),
            Modo::Ventana => self.ventana_hover.filter(Recuadro::vale),
            Modo::Seleccion => self.marcando().filter(Recuadro::vale),
        }
    }

    /// El recuadro que se está viendo: el del arrastre si hay uno, y si no el
    /// último que quedó marcado.
    // 0.1.0
    fn marcando(&self) -> Option<Recuadro> {
        match self.arrastre {
            Some(Arrastre::Nuevo(a, b)) => Some(Recuadro::entre(a, b)),
            Some(Arrastre::Ajuste {
                antes,
                agarre,
                desde,
                hasta,
            }) => Some(antes.ajustado(
                agarre,
                (hasta.0 - desde.0, hasta.1 - desde.1),
                self.pantalla,
            )),
            None => self.hecho,
        }
    }

    /// Lo marcado que se ve: el velo tiene el agujero ahí.
    // 0.1.0
    pub fn marcado(&self) -> Option<Recuadro> {
        self.recuadro()
    }

    /// Esquina de la barra de modos, en lógicos de la pantalla.
    // 0.1.0
    pub fn origen_barra(&self) -> (f32, f32) {
        (self.x_barra(), self.y_barra())
    }

    // 0.1.0
    pub fn tamano_barra(&self) -> (f32, f32) {
        (self.ancho_barra(), BARRA_ALTO)
    }

    /// Esquina del popover de opciones cuando está abierto.
    pub fn origen_opciones(&self) -> Option<(f32, f32)> {
        if !self.popover_abierto {
            return None;
        }
        let (ox, _, _, _) = self.rect_opciones();
        let (pw, ph) = self.tamano_opciones();
        let x = (ox + OPCIONES_W - pw).clamp(8.0, (self.pantalla.0 - pw - 8.0).max(8.0));
        let y = (self.y_barra() - ph - OPCIONES_POPOVER_HUECO).max(8.0);
        Some((x, y))
    }

    pub fn tamano_opciones(&self) -> (f32, f32) {
        let h = match self.tipo {
            TipoCaptura::Foto => OPCIONES_POPOVER_H_FOTO,
            TipoCaptura::Video => OPCIONES_POPOVER_H_VIDEO,
        };
        (OPCIONES_POPOVER_W, h)
    }

    /// Esquina de la pastilla de las medidas.
    // 0.1.0
    pub fn origen_pastilla(&self) -> Option<(f32, f32)> {
        let (pw, ph) = self.pantalla;
        let r = self.medidas().and(self.marcado())?;
        let cabe_debajo = r.y + r.h + PASTILLA_HUECO + PASTILLA_ALTO <= ph;
        let py = if cabe_debajo {
            r.y + r.h + PASTILLA_HUECO
        } else {
            (r.y + r.h - PASTILLA_ALTO - PASTILLA_HUECO).max(0.0)
        };
        let px = (r.x + (r.w - PASTILLA_ANCHO) / 2.0).clamp(0.0, (pw - PASTILLA_ANCHO).max(0.0));
        Some((px, py))
    }

    /// Lo que pone la pastilla.
    // 0.1.0
    pub fn medidas(&self) -> Option<(i32, i32)> {
        match self.modo {
            Modo::Pantalla | Modo::Ventana => None,
            Modo::Seleccion => self.marcado().map(|r| (r.w as i32, r.h as i32)),
        }
    }

    fn y_barra(&self) -> f32 {
        self.pantalla.1 - BARRA_MARGEN - BARRA_ALTO
    }

    fn ancho_tipo() -> f32 {
        SEG_PADDING * 2.0 + TIPO_BTN_W * 2.0 + GAP_SEG
    }

    fn ancho_area() -> f32 {
        SEG_PADDING * 2.0 + AREA_BTN_W * 3.0 + GAP_SEG * 2.0
    }

    fn ancho_barra(&self) -> f32 {
        BARRA_PADDING * 2.0
            + CERRAR_W
            + Self::ancho_tipo()
            + DIVISOR_ANCHO
            + Self::ancho_area()
            + DIVISOR_ANCHO
            + OPCIONES_W
            + ACCION_W
            + GAP_BARRA * 6.0
    }

    fn x_barra(&self) -> f32 {
        (self.pantalla.0 - self.ancho_barra()) / 2.0
    }

    fn rect_cerrar(&self) -> (f32, f32, f32, f32) {
        let x = self.x_barra() + BARRA_PADDING;
        let y = self.y_barra() + BARRA_PADDING;
        (x, y, CERRAR_W, CONTROL_ALTO)
    }

    fn x_tipo_inicio(&self) -> f32 {
        self.x_barra() + BARRA_PADDING + CERRAR_W + GAP_BARRA + SEG_PADDING
    }

    fn x_area_inicio(&self) -> f32 {
        self.x_barra()
            + BARRA_PADDING
            + CERRAR_W
            + GAP_BARRA
            + Self::ancho_tipo()
            + GAP_BARRA
            + DIVISOR_ANCHO
            + GAP_BARRA
            + SEG_PADDING
    }

    fn rect_opciones(&self) -> (f32, f32, f32, f32) {
        let x = self.x_barra()
            + BARRA_PADDING
            + CERRAR_W
            + GAP_BARRA
            + Self::ancho_tipo()
            + GAP_BARRA
            + DIVISOR_ANCHO
            + GAP_BARRA
            + Self::ancho_area()
            + GAP_BARRA
            + DIVISOR_ANCHO
            + GAP_BARRA;
        let y = self.y_barra() + BARRA_PADDING;
        (x, y, OPCIONES_W, CONTROL_ALTO)
    }

    fn rect_accion(&self) -> (f32, f32, f32, f32) {
        let (ox, y, _, _) = self.rect_opciones();
        (ox + OPCIONES_W + GAP_BARRA, y, ACCION_W, CONTROL_ALTO)
    }

    fn celda_en(&self, x: f32, y: f32) -> Option<CeldaBarra> {
        let y_ctrl = self.y_barra() + BARRA_PADDING;
        if y < y_ctrl || y > y_ctrl + CONTROL_ALTO {
            return None;
        }
        let (cx, _, cw, _) = self.rect_cerrar();
        if x >= cx && x < cx + cw {
            return Some(CeldaBarra::Cerrar);
        }
        let y_thumb = y_ctrl + SEG_PADDING;
        if y >= y_thumb && y <= y_thumb + THUMB_ALTO {
            let tx = self.x_tipo_inicio();
            for i in 0..TipoCaptura::TODOS.len() {
                let x0 = tx + i as f32 * (TIPO_BTN_W + GAP_SEG);
                if x >= x0 && x < x0 + TIPO_BTN_W {
                    return Some(CeldaBarra::Tipo(i));
                }
            }
            let ax = self.x_area_inicio();
            for i in 0..Modo::TODOS.len() {
                let x0 = ax + i as f32 * (AREA_BTN_W + GAP_SEG);
                if x >= x0 && x < x0 + AREA_BTN_W {
                    return Some(CeldaBarra::Area(i));
                }
            }
        }
        let (ox, _, ow, _) = self.rect_opciones();
        if x >= ox && x < ox + ow {
            return Some(CeldaBarra::Opciones);
        }
        let (ax, _, aw, _) = self.rect_accion();
        if x >= ax && x < ax + aw {
            return Some(CeldaBarra::Accion);
        }
        None
    }

    fn celda_popover_en(&self, x: f32, y: f32) -> Option<CeldaPopover> {
        let (ox, oy) = self.origen_opciones()?;
        let (pw, ph) = self.tamano_opciones();
        if x < ox || x > ox + pw || y < oy || y > oy + ph {
            return None;
        }
        let lx = x - ox - 12.0;
        let ly = y - oy - 12.0;
        if lx < 0.0 || lx > pw - 24.0 || ly < 0.0 {
            return None;
        }
        if (22.0..=50.0).contains(&ly) {
            let pill_w = (pw - 24.0 - 12.0) / 4.0;
            for i in 0..4 {
                let x0 = i as f32 * (pill_w + 4.0);
                if lx >= x0 && lx <= x0 + pill_w {
                    return Some(CeldaPopover::Retardo(i));
                }
            }
        }
        if (60.0..=96.0).contains(&ly) {
            return Some(CeldaPopover::Cursor);
        }
        if (128.0..=156.0).contains(&ly) {
            match self.tipo {
                TipoCaptura::Foto => {
                    let pill_w = (pw - 24.0 - 6.0) / 2.0;
                    for i in 0..2 {
                        let x0 = i as f32 * (pill_w + 6.0);
                        if lx >= x0 && lx <= x0 + pill_w {
                            return Some(CeldaPopover::Destino(i));
                        }
                    }
                }
                TipoCaptura::Video => {
                    let pill_w = (pw - 24.0 - 12.0) / 4.0;
                    for i in 0..4 {
                        let x0 = i as f32 * (pill_w + 4.0);
                        if lx >= x0 && lx <= x0 + pill_w {
                            return Some(CeldaPopover::Audio(i));
                        }
                    }
                }
            }
        }
        None
    }

    fn en_el_popover(&self, x: f32, y: f32) -> bool {
        let Some((ox, oy)) = self.origen_opciones() else {
            return false;
        };
        let (pw, ph) = self.tamano_opciones();
        x >= ox && x <= ox + pw && y >= oy && y <= oy + ph
    }

    fn en_la_barra(&self, x: f32, y: f32) -> bool {
        x >= self.x_barra()
            && x <= self.x_barra() + self.ancho_barra()
            && y >= self.y_barra()
            && y <= self.y_barra() + BARRA_ALTO
    }

    /// Mueve el puntero. `true` si hay que repintar barra u opciones.
    // 0.1.0
    pub fn puntero(&mut self, x: f32, y: f32) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let señalada = self.celda_en(x, y);
        let señalada_popover = self.celda_popover_en(x, y);
        let cambio_señal = señalada != self.señalada || señalada_popover != self.señalada_popover;
        self.señalada = señalada;
        self.señalada_popover = señalada_popover;
        let punto = (x.clamp(0.0, self.pantalla.0), y.clamp(0.0, self.pantalla.1));
        match self.arrastre.as_mut() {
            Some(Arrastre::Nuevo(_, hasta)) | Some(Arrastre::Ajuste { hasta, .. }) => {
                *hasta = punto
            }
            None => {}
        }
        cambio_señal
    }

    /// La forma del cursor en ese punto.
    // 0.1.0
    pub fn forma(&self, x: f32, y: f32) -> Forma {
        if !x.is_finite() || !y.is_finite() {
            return Forma::Cruz;
        }
        if self.en_la_barra(x, y) || self.en_el_popover(x, y) || self.modo == Modo::Ventana {
            return Forma::Flecha;
        }
        let agarre = match self.arrastre {
            Some(Arrastre::Nuevo(..)) => return Forma::Cruz,
            Some(Arrastre::Ajuste { agarre, .. }) => Some(agarre),
            None => self.hecho_ajustable().and_then(|r| r.agarre(x, y)),
        };
        match agarre {
            None => Forma::Cruz,
            Some(Agarre::Mover) => Forma::Mover,
            Some(Agarre::Estirar { x, y }) => match (x, y) {
                (Lado::Ninguno, _) => Forma::Vertical,
                (_, Lado::Ninguno) => Forma::Horizontal,
                (a, b) if a == b => Forma::DiagonalBajando,
                _ => Forma::DiagonalSubiendo,
            },
        }
    }

    fn hecho_ajustable(&self) -> Option<Recuadro> {
        self.hecho.filter(|_| self.modo == Modo::Seleccion)
    }

    /// Empieza un arrastre o pulsa un botón de la barra/opciones.
    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        if self.popover_abierto {
            if let Some(celda) = self.celda_popover_en(x, y) {
                match celda {
                    CeldaPopover::Retardo(i) => {
                        self.retardo = crate::config::RETARDOS_CAPTURA[i];
                        return Some(Accion::CapturaOpciones {
                            cursor: self.cursor,
                            retardo: self.retardo,
                        });
                    }
                    CeldaPopover::Cursor => {
                        self.cursor = !self.cursor;
                        return Some(Accion::CapturaOpciones {
                            cursor: self.cursor,
                            retardo: self.retardo,
                        });
                    }
                    CeldaPopover::Destino(i) => {
                        self.destino = Destino::TODOS[i];
                        return None;
                    }
                    CeldaPopover::Audio(i) => {
                        self.audio = AudioGrabacion::TODOS[i];
                        return None;
                    }
                }
            }
            if self.en_el_popover(x, y) {
                return None;
            }
        }
        if let Some(celda) = self.celda_en(x, y) {
            if celda != CeldaBarra::Opciones {
                self.popover_abierto = false;
            }
            match celda {
                CeldaBarra::Cerrar => return Some(Accion::CapturaCerrar),
                CeldaBarra::Tipo(i) => {
                    self.tipo = TipoCaptura::TODOS[i];
                    return None;
                }
                CeldaBarra::Area(i) => {
                    self.arrastre = None;
                    self.modo = Modo::TODOS[i];
                    return None;
                }
                CeldaBarra::Opciones => {
                    self.popover_abierto = !self.popover_abierto;
                    return None;
                }
                CeldaBarra::Accion => return self.ejecutar_accion(),
            }
        }
        if self.en_la_barra(x, y) {
            return None;
        }
        self.popover_abierto = false;
        if self.modo == Modo::Ventana {
            return self.ejecutar_accion();
        }
        let punto = (x.clamp(0.0, self.pantalla.0), y.clamp(0.0, self.pantalla.1));
        if let Some(antes) = self.hecho_ajustable()
            && let Some(agarre) = antes.agarre(x, y)
        {
            self.arrastre = Some(Arrastre::Ajuste {
                antes,
                agarre,
                desde: punto,
                hasta: punto,
            });
            return None;
        }
        self.modo = Modo::Seleccion;
        self.arrastre = Some(Arrastre::Nuevo(punto, punto));
        self.hecho = None;
        None
    }

    pub fn soltar(&mut self) {
        let Some(arrastre) = self.arrastre else {
            return;
        };
        let nuevo = self.marcando().filter(Recuadro::vale);
        self.arrastre = None;
        self.hecho = match arrastre {
            Arrastre::Nuevo(..) => nuevo,
            Arrastre::Ajuste { antes, .. } => nuevo.or(Some(antes)),
        };
    }

    pub fn cancelar_arrastre(&mut self) {
        let Some(arrastre) = self.arrastre.take() else {
            return;
        };
        self.hecho = match arrastre {
            Arrastre::Nuevo(..) => None,
            Arrastre::Ajuste { antes, .. } => Some(antes),
        };
    }

    fn ejecutar_accion(&self) -> Option<Accion> {
        let r = self.recuadro()?;
        match self.tipo {
            TipoCaptura::Foto => Some(Accion::Capturar {
                x: r.x as i32,
                y: r.y as i32,
                ancho: r.w as i32,
                alto: r.h as i32,
                guardar: self.destino == Destino::Guardar,
                cursor: self.cursor,
                retardo: self.retardo,
            }),
            TipoCaptura::Video => Some(Accion::GrabarIniciar {
                x: r.x as i32,
                y: r.y as i32,
                ancho: r.w as i32,
                alto: r.h as i32,
                cursor: self.cursor,
                retardo: self.retardo,
                audio: self.audio,
            }),
        }
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> crate::Tecla {
        use crate::Tecla;
        match tecla {
            TeclaPulsada::Escape => {
                if self.popover_abierto {
                    self.popover_abierto = false;
                    Tecla::Consumida
                } else {
                    Tecla::Cerrar
                }
            }
            TeclaPulsada::Intro => {
                if self.recuadro().is_none() {
                    self.modo = Modo::Pantalla;
                }
                match self.ejecutar_accion() {
                    Some(accion) => Tecla::Hacer(accion),
                    None => Tecla::Consumida,
                }
            }
            TeclaPulsada::Izquierda | TeclaPulsada::Derecha => {
                let i = Modo::TODOS
                    .iter()
                    .position(|m| *m == self.modo)
                    .unwrap_or(0);
                let paso = if matches!(tecla, TeclaPulsada::Derecha) {
                    1
                } else {
                    Modo::TODOS.len() - 1
                };
                self.arrastre = None;
                self.modo = Modo::TODOS[(i + paso) % Modo::TODOS.len()];
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    pub fn animando(&self) -> bool {
        self.arrastre.is_some()
    }

    pub fn vista_pastilla(&self) -> PanelElement<'_> {
        let (w, h) = self.medidas().unwrap_or_default();
        container(
            text(format!("{w} × {h}"))
                .size(tema::T_PEQUENO)
                .color(tema::texto()),
        )
        .width(Length::Fixed(PASTILLA_ANCHO))
        .height(Length::Fixed(PASTILLA_ALTO))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(|_| iced_widget::container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Barra flotante inferior 1:1 con el diseño HIG:
    /// `[×] [Foto | Vídeo] | [Pantalla | Ventana | Selección] | [Opciones] [Capturar/Grabar]`
    pub fn vista_barra(&self) -> PanelElement<'_> {
        let thumb_fondo = if tema::es_claro() {
            iced_core::Color::WHITE
        } else {
            tema::alfa(iced_core::Color::WHITE, 0.16)
        };
        let track_fondo = tema::alfa(tema::tinta(), 0.08);
        let divisor = || {
            container(Space::new())
                .width(Length::Fixed(DIVISOR_ANCHO))
                .height(Length::Fixed(22.0))
                .style(|_| iced_widget::container::Style {
                    background: Some(tema::alfa(tema::tinta(), 0.08).into()),
                    ..Default::default()
                })
        };

        let cerrar_señal = self.señalada == Some(CeldaBarra::Cerrar);
        let btn_cerrar = container(match crate::icono::propio("cerrar") {
            Some(ic) => crate::icono::ver_teñido_propio(
                &ic,
                15.0,
                if cerrar_señal { tema::texto() } else { tema::texto2() },
            ),
            None => Space::new().width(Length::Fixed(15.0)).into(),
        })
        .width(Length::Fixed(CERRAR_W))
        .height(Length::Fixed(CONTROL_ALTO))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| iced_widget::container::Style {
            background: Some(if cerrar_señal { tema::hover() } else { track_fondo }.into()),
            border: Border {
                radius: CONTROL_RADIO.into(),
                width: 1.0,
                color: tema::borde(),
            },
            ..Default::default()
        });

        let mut seg_tipo = row![].spacing(GAP_SEG);
        for (i, t) in TipoCaptura::TODOS.iter().enumerate() {
            let activa = *t == self.tipo;
            let señal = self.señalada == Some(CeldaBarra::Tipo(i));
            let color_txt = if activa || señal {
                tema::texto()
            } else {
                tema::texto2()
            };
            seg_tipo = seg_tipo.push(
                container(
                    row![
                        match crate::icono::propio(t.simbolo()) {
                            Some(ic) => crate::icono::ver_teñido_propio(&ic, 15.0, color_txt),
                            None => Space::new().width(Length::Fixed(15.0)).into(),
                        },
                        text(t.nombre()).size(tema::T_CUERPO).color(color_txt),
                    ]
                    .spacing(6.0)
                    .align_y(Vertical::Center),
                )
                .width(Length::Fixed(TIPO_BTN_W))
                .height(Length::Fixed(THUMB_ALTO))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| iced_widget::container::Style {
                    background: Some(
                        if activa {
                            thumb_fondo
                        } else {
                            iced_core::Color::TRANSPARENT
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: THUMB_RADIO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        let track_tipo = container(seg_tipo)
            .padding(SEG_PADDING)
            .height(Length::Fixed(CONTROL_ALTO))
            .style(move |_| iced_widget::container::Style {
                background: Some(track_fondo.into()),
                border: Border {
                    radius: CONTROL_RADIO.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let mut seg_area = row![].spacing(GAP_SEG);
        for (i, m) in Modo::TODOS.iter().enumerate() {
            let activa = *m == self.modo;
            let señal = self.señalada == Some(CeldaBarra::Area(i));
            let color_txt = if activa || señal {
                tema::texto()
            } else {
                tema::texto2()
            };
            seg_area = seg_area.push(
                container(
                    row![
                        match crate::icono::propio(m.simbolo()) {
                            Some(ic) => crate::icono::ver_teñido_propio(&ic, 15.0, color_txt),
                            None => Space::new().width(Length::Fixed(15.0)).into(),
                        },
                        text(m.nombre()).size(tema::T_CUERPO).color(color_txt),
                    ]
                    .spacing(6.0)
                    .align_y(Vertical::Center),
                )
                .width(Length::Fixed(AREA_BTN_W))
                .height(Length::Fixed(THUMB_ALTO))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| iced_widget::container::Style {
                    background: Some(
                        if activa {
                            thumb_fondo
                        } else {
                            iced_core::Color::TRANSPARENT
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: THUMB_RADIO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        let track_area = container(seg_area)
            .padding(SEG_PADDING)
            .height(Length::Fixed(CONTROL_ALTO))
            .style(move |_| iced_widget::container::Style {
                background: Some(track_fondo.into()),
                border: Border {
                    radius: CONTROL_RADIO.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let opt_señal = self.señalada == Some(CeldaBarra::Opciones) || self.popover_abierto;
        let btn_opciones = container(
            row![
                match crate::icono::propio("control") {
                    Some(ic) => crate::icono::ver_teñido_propio(&ic, 15.0, tema::texto()),
                    None => Space::new().width(Length::Fixed(15.0)).into(),
                },
                text(crate::emergente::t("Opciones", "Options"))
                    .size(tema::T_CUERPO)
                    .color(tema::texto()),
            ]
            .spacing(6.0)
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(OPCIONES_W))
        .height(Length::Fixed(CONTROL_ALTO))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| iced_widget::container::Style {
            background: Some(if opt_señal { tema::hover() } else { track_fondo }.into()),
            border: Border {
                radius: CONTROL_RADIO.into(),
                width: 1.0,
                color: tema::borde(),
            },
            ..Default::default()
        });

        let puede = self.recuadro().is_some();
        let etiqueta_accion = match self.tipo {
            TipoCaptura::Foto => match self.destino {
                Destino::Portapapeles => crate::emergente::t("Copiar", "Copy").to_string(),
                Destino::Guardar => crate::emergente::t("Capturar", "Capture").to_string(),
            },
            TipoCaptura::Video => {
                if self.retardo > 0 {
                    format!("{} ({}s)", crate::emergente::t("Grabar", "Record"), self.retardo)
                } else {
                    crate::emergente::t("Grabar", "Record").to_string()
                }
            }
        };
        let color_accion = match self.tipo {
            TipoCaptura::Foto => tema::acento(),
            TipoCaptura::Video => tema::rojo(),
        };
        let fondo_accion = if puede {
            color_accion
        } else {
            tema::alfa(color_accion, 0.45)
        };
        let btn_accion = container(
            text(etiqueta_accion)
                .size(tema::T_CUERPO)
                .color(iced_core::Color::WHITE),
        )
        .width(Length::Fixed(ACCION_W))
        .height(Length::Fixed(CONTROL_ALTO))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| iced_widget::container::Style {
            background: Some(fondo_accion.into()),
            border: Border {
                radius: CONTROL_RADIO.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let fila = row![
            btn_cerrar,
            track_tipo,
            divisor(),
            track_area,
            divisor(),
            btn_opciones,
            btn_accion,
        ]
        .spacing(GAP_BARRA)
        .align_y(Vertical::Center);

        container(fila)
            .padding(BARRA_PADDING)
            .width(Length::Fixed(self.ancho_barra()))
            .height(Length::Fixed(BARRA_ALTO))
            .style(|_| iced_widget::container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: BARRA_RADIO.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }

    /// Popover flotante de opciones encima del botón «Opciones».
    pub fn vista_opciones(&self) -> PanelElement<'_> {
        let (_, ph) = self.tamano_opciones();
        let pill = |etiqueta: String, activa: bool, señal: bool, w: f32| -> PanelElement<'_> {
            container(
                text(etiqueta)
                    .size(tema::T_PEQUENO)
                    .color(if activa {
                        iced_core::Color::WHITE
                    } else if señal {
                        tema::texto()
                    } else {
                        tema::texto2()
                    }),
            )
            .width(Length::Fixed(w))
            .height(Length::Fixed(28.0))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_| iced_widget::container::Style {
                background: Some(
                    if activa {
                        tema::acento()
                    } else if señal {
                        tema::hover()
                    } else {
                        tema::alfa(tema::tinta(), 0.08)
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        };

        let mut retardos = row![].spacing(4.0);
        let pill_w4 = (OPCIONES_POPOVER_W - 24.0 - 12.0) / 4.0;
        for (i, &r) in crate::config::RETARDOS_CAPTURA.iter().enumerate() {
            let lbl = if r == 0 {
                crate::emergente::t("Sin espera", "None").to_string()
            } else {
                format!("{r} s")
            };
            retardos = retardos.push(pill(
                lbl,
                self.retardo == r,
                self.señalada_popover == Some(CeldaPopover::Retardo(i)),
                pill_w4,
            ));
        }

        let cursor_señal = self.señalada_popover == Some(CeldaPopover::Cursor);
        let toggle = container(
            container(Space::new())
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .style(|_| iced_widget::container::Style {
                    background: Some(iced_core::Color::WHITE.into()),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(42.0))
        .height(Length::Fixed(24.0))
        .padding(3.0)
        .align_x(if self.cursor {
            Horizontal::Right
        } else {
            Horizontal::Left
        })
        .align_y(Vertical::Center)
        .style(|_| iced_widget::container::Style {
            background: Some(
                if self.cursor {
                    tema::acento()
                } else {
                    tema::alfa(tema::tinta(), 0.22)
                }
                .into(),
            ),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let fila_cursor = container(
            row![
                text(crate::emergente::t("Mostrar puntero", "Show cursor"))
                    .size(tema::T_CUERPO)
                    .color(tema::texto()),
                Space::new().width(Length::Fill),
                toggle,
            ]
            .align_y(Vertical::Center),
        )
        .height(Length::Fixed(36.0))
        .padding([0.0, 6.0])
        .align_y(Vertical::Center)
        .style(move |_| iced_widget::container::Style {
            background: Some(
                if cursor_señal {
                    tema::hover()
                } else {
                    iced_core::Color::TRANSPARENT
                }
                .into(),
            ),
            border: Border {
                radius: tema::R_ITEM_POPOVER.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let (subtitulo_3, grupo_3): (&str, PanelElement<'_>) = match self.tipo {
            TipoCaptura::Foto => {
                let mut dest = row![].spacing(6.0);
                let w2 = (OPCIONES_POPOVER_W - 24.0 - 6.0) / 2.0;
                for (i, d) in Destino::TODOS.iter().enumerate() {
                    let lbl = match d {
                        Destino::Portapapeles => crate::emergente::t("Portapapeles", "Clipboard"),
                        Destino::Guardar => crate::emergente::t("Imágenes", "Pictures"),
                    };
                    dest = dest.push(pill(
                        lbl.to_string(),
                        *d == self.destino,
                        self.señalada_popover == Some(CeldaPopover::Destino(i)),
                        w2,
                    ));
                }
                (crate::emergente::t("Guardar en", "Save to"), dest.into())
            }
            TipoCaptura::Video => {
                let mut aud = row![].spacing(4.0);
                for (i, a) in AudioGrabacion::TODOS.iter().enumerate() {
                    aud = aud.push(pill(
                        a.nombre().to_string(),
                        *a == self.audio,
                        self.señalada_popover == Some(CeldaPopover::Audio(i)),
                        pill_w4,
                    ));
                }
                (crate::emergente::t("Audio", "Audio"), aud.into())
            }
        };

        let subtitulo = |etiqueta: &str| {
            container(
                text(etiqueta.to_string())
                    .size(tema::T_PEQUENO)
                    .color(tema::texto2()),
            )
            .height(Length::Fixed(16.0))
            .align_y(Vertical::Center)
        };

        let contenido = column![
            subtitulo(crate::emergente::t("Temporizador", "Timer")),
            Space::new().height(Length::Fixed(6.0)),
            retardos,
            Space::new().height(Length::Fixed(10.0)),
            fila_cursor,
            Space::new().height(Length::Fixed(10.0)),
            subtitulo(subtitulo_3),
            Space::new().height(Length::Fixed(6.0)),
            grupo_3,
        ];

        container(contenido)
            .padding(12.0)
            .width(Length::Fixed(OPCIONES_POPOVER_W))
            .height(Length::Fixed(ph))
            .style(|_| iced_widget::container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }
}

/// Pill flotante durante la grabación de pantalla.
///
/// El compositor lo excluye de la escena cuando compone fotogramas para
/// grabación/emisión, así que no aparece en el vídeo resultante.
pub struct GrabacionPill {
    pantalla: (f32, f32),
    origen: (f32, f32),
    arrastre: Option<((f32, f32), (f32, f32))>,
    inicio: std::time::Instant,
    pausado: bool,
    acumulado: std::time::Duration,
    silenciado: bool,
    tiene_mic: bool,
    señalado: Option<usize>,
}

impl GrabacionPill {
    pub fn new(pantalla: (f32, f32), tiene_mic: bool) -> Self {
        let origen = (
            (pantalla.0 - PILL_GRABACION_W - 20.0).max(12.0),
            48.0,
        );
        Self {
            pantalla,
            origen,
            arrastre: None,
            inicio: std::time::Instant::now(),
            pausado: false,
            acumulado: std::time::Duration::ZERO,
            silenciado: false,
            tiene_mic,
            señalado: None,
        }
    }

    pub fn origen(&self) -> (f32, f32) {
        self.origen
    }

    pub fn tamano(&self) -> (f32, f32) {
        (PILL_GRABACION_W, PILL_GRABACION_H)
    }

    pub fn segundos(&self) -> u64 {
        let extra = if self.pausado {
            std::time::Duration::ZERO
        } else {
            self.inicio.elapsed()
        };
        (self.acumulado + extra).as_secs()
    }

    pub fn contiene(&self, x: f32, y: f32) -> bool {
        x >= self.origen.0
            && x <= self.origen.0 + PILL_GRABACION_W
            && y >= self.origen.1
            && y <= self.origen.1 + PILL_GRABACION_H
    }

    fn boton_en(&self, x: f32, y: f32) -> Option<usize> {
        if !self.contiene(x, y) {
            return None;
        }
        let lx = x - self.origen.0;
        let ly = y - self.origen.1;
        if !(6.0..=36.0).contains(&ly) {
            return None;
        }
        if (78.0..=106.0).contains(&lx) && self.tiene_mic {
            Some(0)
        } else if (112.0..=140.0).contains(&lx) {
            Some(1)
        } else if (146.0..=244.0).contains(&lx) {
            Some(2)
        } else {
            None
        }
    }

    pub fn puntero(&mut self, x: f32, y: f32) -> bool {
        if let Some((desde_puntero, desde_origen)) = self.arrastre {
            let nx = (desde_origen.0 + x - desde_puntero.0)
                .clamp(12.0, (self.pantalla.0 - PILL_GRABACION_W - 12.0).max(12.0));
            let ny = (desde_origen.1 + y - desde_puntero.1)
                .clamp(40.0, (self.pantalla.1 - PILL_GRABACION_H - 12.0).max(40.0));
            let cambio = self.origen != (nx, ny);
            self.origen = (nx, ny);
            return cambio;
        }
        let nuevo = self.boton_en(x, y);
        let cambio = nuevo != self.señalado;
        self.señalado = nuevo;
        cambio
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if !self.contiene(x, y) {
            return None;
        }
        match self.boton_en(x, y) {
            Some(0) => {
                self.silenciado = !self.silenciado;
                Some(Accion::GrabarSilenciar(self.silenciado))
            }
            Some(1) => {
                if self.pausado {
                    self.inicio = std::time::Instant::now();
                    self.pausado = false;
                } else {
                    self.acumulado += self.inicio.elapsed();
                    self.pausado = true;
                }
                Some(Accion::GrabarPausar(self.pausado))
            }
            Some(2) => Some(Accion::GrabarDetener),
            _ => {
                self.arrastre = Some(((x, y), self.origen));
                None
            }
        }
    }

    pub fn soltar(&mut self) {
        self.arrastre = None;
    }

    pub fn vista(&self) -> PanelElement<'_> {
        let segs = self.segundos();
        let tiempo = format!("{:02}:{:02}", segs / 60, segs % 60);
        let punto_color = if self.pausado {
            iced_core::Color::from_rgb(1.0, 0.624, 0.039)
        } else {
            tema::rojo()
        };
        let punto = container(Space::new())
            .width(Length::Fixed(9.0))
            .height(Length::Fixed(9.0))
            .style(move |_| iced_widget::container::Style {
                background: Some(punto_color.into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let icono_btn = |nombre: &str, señal: bool, acento_naranja: bool| -> PanelElement<'_> {
            let color = if acento_naranja {
                iced_core::Color::from_rgb(1.0, 0.624, 0.039)
            } else {
                tema::texto()
            };
            container(match crate::icono::propio(nombre) {
                Some(ic) => crate::icono::ver_teñido_propio(&ic, 15.0, color),
                None => Space::new().width(Length::Fixed(15.0)).into(),
            })
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(28.0))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_| iced_widget::container::Style {
                background: Some(
                    if señal {
                        tema::hover()
                    } else {
                        tema::alfa(tema::tinta(), 0.08)
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        };

        let mut fila = row![
            punto,
            container(text(tiempo).size(tema::T_CUERPO).color(tema::texto()))
                .width(Length::Fixed(49.0))
                .align_y(Vertical::Center),
        ]
        .spacing(6.0)
        .align_y(Vertical::Center);

        if self.tiene_mic {
            fila = fila.push(icono_btn(
                if self.silenciado { "micro-silencio" } else { "micro" },
                self.señalado == Some(0),
                self.silenciado,
            ));
        } else {
            fila = fila.push(Space::new().width(Length::Fixed(28.0)));
        }

        fila = fila.push(icono_btn(
            if self.pausado { "reproducir" } else { "pausa" },
            self.señalado == Some(1),
            self.pausado,
        ));

        let btn_detener = container(
            row![
                container(Space::new())
                    .width(Length::Fixed(9.0))
                    .height(Length::Fixed(9.0))
                    .style(|_| iced_widget::container::Style {
                        background: Some(iced_core::Color::WHITE.into()),
                        border: Border {
                            radius: 2.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                text(crate::emergente::t("Detener", "Stop"))
                    .size(tema::T_PEQUENO)
                    .color(iced_core::Color::WHITE),
            ]
            .spacing(6.0)
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(98.0))
        .height(Length::Fixed(28.0))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(|_| iced_widget::container::Style {
            background: Some(tema::rojo().into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        fila = fila.push(btn_detener);

        container(fila)
            .padding([6.0, 8.0])
            .width(Length::Fixed(PILL_GRABACION_W))
            .height(Length::Fixed(PILL_GRABACION_H))
            .align_y(Vertical::Center)
            .style(|_| iced_widget::container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }
}

/// Reserva una ruta única en disco para la próxima captura sin codificar aún el PNG.
pub fn reservar_ruta_png() -> std::io::Result<std::path::PathBuf> {
    let base = carpeta_de_capturas();
    std::fs::create_dir_all(&base)?;
    let marca = sello_de_tiempo();
    let directa = base.join(format!("Captura {marca}.png"));
    if !directa.exists() {
        return Ok(directa);
    }
    for n in 2..1000 {
        let candidata = base.join(format!("Captura {marca} ({n}).png"));
        if !candidata.exists() {
            return Ok(candidata);
        }
    }
    Ok(base.join(format!("Captura {marca}-{}.png", std::process::id())))
}

/// Escribe bytes PNG ya codificados de forma atómica (`.part` + rename) para
/// no recodificar ni exponer ficheros a medio escribir al visor.
pub fn guardar_bytes_png(
    png: &[u8],
    destino: Option<std::path::PathBuf>,
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write as _;

    let ruta = match destino {
        Some(r) => {
            if let Some(dir) = r.parent() {
                std::fs::create_dir_all(dir)?;
            }
            r
        }
        None => reservar_ruta_png()?,
    };
    let nombre = ruta
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "captura.png".into());
    let temporal = ruta.with_file_name(format!(".{nombre}.{}.part", std::process::id()));
    let escrito = (|| -> std::io::Result<()> {
        let mut fichero = std::fs::File::create(&temporal)?;
        fichero.write_all(png)?;
        fichero.flush()?;
        std::fs::rename(&temporal, &ruta)?;
        Ok(())
    })();
    if let Err(err) = escrito {
        let _ = std::fs::remove_file(&temporal);
        return Err(err);
    }
    if let Ok(mut guard) = ULTIMA.lock() {
        *guard = Some(ruta.clone());
    }
    Ok(ruta)
}

/// Guarda una captura en `destino` (o una ruta nueva si es `None`) usando
/// codificación rápida y escritura atómica.
pub fn guardar_png_en(
    rgba: &[u8],
    ancho: u32,
    alto: u32,
    destino: Option<std::path::PathBuf>,
) -> std::io::Result<std::path::PathBuf> {
    let png = png_en_memoria(rgba, ancho, alto)?;
    guardar_bytes_png(&png, destino)
}

/// Guarda una captura en el disco y devuelve dónde quedó.
///
/// En `~/Imágenes/Capturas` —o `$XDG_PICTURES_DIR/Capturas`—, con el nombre
/// `Captura 2026-08-24 a las 10.42.31.png`: la fecha delante para que ordenen
/// solas por nombre, y con puntos en la hora porque los dos puntos no se pueden
/// escribir en un nombre de fichero en todos los sistemas de archivos.
// 0.1.0
pub fn guardar_png(rgba: &[u8], ancho: u32, alto: u32) -> std::io::Result<std::path::PathBuf> {
    guardar_png_en(rgba, ancho, alto, None)
}

/// La misma captura, pero en memoria: es lo que se pone en el portapapeles.
// 0.1.0
pub fn png_en_memoria(rgba: &[u8], ancho: u32, alto: u32) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    codificar_png(std::io::Cursor::new(&mut bytes), rgba, ancho, alto)?;
    Ok(bytes)
}

// 0.1.0
// modified 0.2.4 -- compresión Fast + filtro Sub para minimizar la latencia al capturar y abrir
fn codificar_png<W: std::io::Write>(
    destino: W,
    rgba: &[u8],
    ancho: u32,
    alto: u32,
) -> std::io::Result<()> {
    // El mismo `image` que ya usa el shell para los avatares y los fondos: vive
    // aquí y no en el compositor para no añadirle una dependencia que ya está a
    // un crate de distancia.
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    PngEncoder::new_with_quality(destino, CompressionType::Fast, FilterType::Sub)
        .write_image(rgba, ancho, alto, image::ExtendedColorType::Rgba8)
        .map_err(std::io::Error::other)
}

/// La última captura guardada en esta sesión.
///
/// Existe solo para el autotest: es la única forma de comprobar desde dentro
/// que el fichero acabó en el disco sin adivinar su nombre, que lleva la hora.
static ULTIMA: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

// 0.1.0
pub fn ultima_guardada() -> Option<std::path::PathBuf> {
    ULTIMA.lock().ok().and_then(|g| g.clone())
}

/// Dónde van las capturas.
// 0.1.0
fn carpeta_de_capturas() -> std::path::PathBuf {
    // `XDG_PICTURES_DIR` la pone `xdg-user-dirs`; si no está, se cae a la
    // carpeta en castellano y luego a la inglesa, que es lo que hay en un
    // sistema recién instalado según el idioma con el que se creó.
    if let Some(dir) = std::env::var_os("XDG_PICTURES_DIR").filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(dir).join("Capturas");
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    for nombre in ["Imágenes", "Pictures"] {
        let candidata = home.join(nombre);
        if candidata.is_dir() {
            return candidata.join("Capturas");
        }
    }
    home.join("Imágenes/Capturas")
}

/// `2026-08-24 a las 10.42.31`, en hora local.
// 0.1.0
fn sello_de_tiempo() -> String {
    let (h, m) = crate::hora_local_ahora();
    let hoy = crate::state::Fecha::hoy();
    // Los segundos no salen de `localtime`: el reloj de pared y el del
    // calendario son el mismo, así que basta el resto de la hora en curso.
    let segundos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() % 60)
        .unwrap_or(0);
    format!(
        "{:04}-{:02}-{:02} a las {:02}.{:02}.{:02}",
        hoy.anio, hoy.mes, hoy.dia, h, m, segundos
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANTALLA: (f32, f32) = (1920.0, 1080.0);

    /// El recuadro sale con lados positivos se arrastre hacia donde se
    /// arrastre: de arriba a la izquierda hacia abajo a la derecha y al revés.
    #[test]
    fn el_recuadro_no_depende_de_la_direccion() {
        let a = Recuadro::entre((100.0, 100.0), (300.0, 250.0));
        let b = Recuadro::entre((300.0, 250.0), (100.0, 100.0));
        assert_eq!(a, b);
        assert_eq!((a.x, a.y, a.w, a.h), (100.0, 100.0, 200.0, 150.0));
    }

    /// Un clic suelto no es una selección: sin el umbral, pulsar para cancelar
    /// guardaba una captura de tres píxeles.
    #[test]
    fn un_clic_suelto_no_captura() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(400.0, 400.0);
        c.puntero(402.0, 401.0);
        c.soltar();
        assert_eq!(c.recuadro(), None, "no puede quedar nada marcado");
    }

    /// Arrastrar marca, y soltar **no** captura: el recuadro se queda marcado
    /// hasta que se elige qué hacer con él.
    #[test]
    fn soltar_deja_marcado_y_no_captura() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        let r = c.recuadro().expect("hay recuadro mientras se arrastra");
        assert_eq!((r.w, r.h), (400.0, 300.0));
        c.soltar();
        assert_eq!(c.recuadro(), Some(r), "lo marcado sigue ahí al soltar");
        assert!(!c.animando());
    }

    /// Marca el recuadro (200,150)-(600,450) y lo suelta.
    fn marcada() -> Captura {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        c.soltar();
        c
    }

    fn medidas(c: &Captura) -> (f32, f32, f32, f32) {
        let r = c.recuadro().expect("hay algo marcado");
        (r.x, r.y, r.w, r.h)
    }

    /// Tirar de la arista de la derecha cambia solo el ancho; de la de abajo,
    /// solo el alto; de una esquina, los dos.
    #[test]
    fn las_aristas_ajustan_ancho_y_alto() {
        let mut c = marcada();
        // Relativo a donde se cogió: la arista no salta al cursor.
        c.pulsar(602.0, 300.0);
        c.puntero(702.0, 280.0);
        c.soltar();
        assert_eq!(medidas(&c), (200.0, 150.0, 500.0, 300.0));

        c.pulsar(400.0, 448.0);
        c.puntero(400.0, 498.0);
        c.soltar();
        assert_eq!(medidas(&c), (200.0, 150.0, 500.0, 350.0));

        // La esquina de arriba a la izquierda mueve el origen y encoge.
        c.pulsar(200.0, 150.0);
        c.puntero(250.0, 200.0);
        c.soltar();
        assert_eq!(medidas(&c), (250.0, 200.0, 450.0, 300.0));
    }

    /// Por dentro se mueve entero, sin cambiar de tamaño y sin salirse.
    #[test]
    fn por_dentro_se_mueve() {
        let mut c = marcada();
        c.pulsar(400.0, 300.0);
        c.puntero(450.0, 320.0);
        assert_eq!(medidas(&c), (250.0, 170.0, 400.0, 300.0));
        c.puntero(-5000.0, -5000.0);
        c.soltar();
        assert_eq!(medidas(&c), (0.0, 0.0, 400.0, 300.0));
    }

    /// Fuera del recuadro se empieza uno nuevo, como antes.
    #[test]
    fn fuera_se_marca_otro() {
        let mut c = marcada();
        c.pulsar(1000.0, 700.0);
        c.puntero(1100.0, 800.0);
        c.soltar();
        assert_eq!(medidas(&c), (1000.0, 700.0, 100.0, 100.0));
    }

    /// Encogerlo hasta nada no borra lo marcado: vuelve a como estaba.
    #[test]
    fn encoger_de_mas_no_lo_borra() {
        let mut c = marcada();
        c.pulsar(600.0, 300.0);
        c.puntero(203.0, 300.0);
        c.soltar();
        assert_eq!(medidas(&c), (200.0, 150.0, 400.0, 300.0));
    }

    #[test]
    fn el_cursor_dice_que_hara_el_clic() {
        let c = marcada();
        assert_eq!(c.forma(600.0, 300.0), Forma::Horizontal);
        assert_eq!(c.forma(400.0, 150.0), Forma::Vertical);
        assert_eq!(c.forma(200.0, 150.0), Forma::DiagonalBajando);
        assert_eq!(c.forma(600.0, 150.0), Forma::DiagonalSubiendo);
        assert_eq!(c.forma(400.0, 300.0), Forma::Mover);
        assert_eq!(c.forma(1000.0, 700.0), Forma::Cruz);
    }

    fn centro_area(c: &Captura, i: usize) -> (f32, f32) {
        (
            c.x_area_inicio() + i as f32 * (AREA_BTN_W + GAP_SEG) + AREA_BTN_W / 2.0,
            c.y_barra() + BARRA_PADDING + CONTROL_ALTO / 2.0,
        )
    }

    fn centro_tipo(c: &Captura, i: usize) -> (f32, f32) {
        (
            c.x_tipo_inicio() + i as f32 * (TIPO_BTN_W + GAP_SEG) + TIPO_BTN_W / 2.0,
            c.y_barra() + BARRA_PADDING + CONTROL_ALTO / 2.0,
        )
    }

    fn centro_opciones(c: &Captura) -> (f32, f32) {
        let (ox, oy, ow, oh) = c.rect_opciones();
        (ox + ow / 2.0, oy + oh / 2.0)
    }

    fn centro_accion(c: &Captura) -> (f32, f32) {
        let (ax, ay, aw, ah) = c.rect_accion();
        (ax + aw / 2.0, ay + ah / 2.0)
    }

    #[test]
    fn el_arrastre_no_se_sale_de_la_pantalla() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(100.0, 100.0);
        c.puntero(5000.0, 5000.0);
        let r = c.recuadro().unwrap();
        assert_eq!((r.x + r.w, r.y + r.h), (PANTALLA.0, PANTALLA.1));
    }

    #[test]
    fn cada_celda_de_la_barra_responde_en_su_sitio() {
        let c = Captura::new(PANTALLA);
        let (cx, cy, cw, ch) = c.rect_cerrar();
        assert_eq!(c.celda_en(cx + cw / 2.0, cy + ch / 2.0), Some(CeldaBarra::Cerrar));
        for i in 0..TipoCaptura::TODOS.len() {
            let (tx, ty) = centro_tipo(&c, i);
            assert_eq!(c.celda_en(tx, ty), Some(CeldaBarra::Tipo(i)));
        }
        for i in 0..Modo::TODOS.len() {
            let (ax, ay) = centro_area(&c, i);
            assert_eq!(c.celda_en(ax, ay), Some(CeldaBarra::Area(i)));
        }
        let (ox, oy) = centro_opciones(&c);
        assert_eq!(c.celda_en(ox, oy), Some(CeldaBarra::Opciones));
        let (ax, ay) = centro_accion(&c);
        assert_eq!(c.celda_en(ax, ay), Some(CeldaBarra::Accion));
        assert!(c.x_barra() > 0.0);
        assert!(c.x_barra() + c.ancho_barra() < PANTALLA.0);
    }

    #[test]
    fn copiar_y_guardar_capturan_lo_marcado() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        c.soltar();
        let (ax, ay) = centro_accion(&c);
        assert_eq!(
            c.pulsar(ax, ay),
            Some(Accion::Capturar {
                x: 200,
                y: 150,
                ancho: 400,
                alto: 300,
                guardar: false,
                cursor: false,
                retardo: 0,
            })
        );
        // Abrir Opciones y elegir "Guardar en Imágenes"
        let (ox, oy) = centro_opciones(&c);
        c.pulsar(ox, oy);
        let (px, py) = c.origen_opciones().unwrap();
        let pill_w2 = (OPCIONES_POPOVER_W - 24.0 - 6.0) / 2.0;
        c.pulsar(px + 12.0 + pill_w2 + 6.0 + pill_w2 / 2.0, py + 12.0 + 150.0);
        assert_eq!(
            c.pulsar(ax, ay),
            Some(Accion::Capturar {
                x: 200,
                y: 150,
                ancho: 400,
                alto: 300,
                guardar: true,
                cursor: false,
                retardo: 0,
            })
        );
    }

    #[test]
    fn el_cursor_y_la_espera_viajan_con_la_captura() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        c.soltar();
        let (ox, oy) = centro_opciones(&c);
        c.pulsar(ox, oy);
        let (px, py) = c.origen_opciones().unwrap();
        let pill_w4 = (OPCIONES_POPOVER_W - 24.0 - 12.0) / 4.0;
        // Activar cursor
        assert_eq!(
            c.pulsar(px + 40.0, py + 12.0 + 82.0),
            Some(Accion::CapturaOpciones { cursor: true, retardo: 0 })
        );
        // Elegir 3 s
        assert_eq!(
            c.pulsar(px + 12.0 + (pill_w4 + 4.0) + pill_w4 / 2.0, py + 12.0 + 36.0),
            Some(Accion::CapturaOpciones { cursor: true, retardo: 3 })
        );
        let (ax, ay) = centro_accion(&c);
        assert!(matches!(
            c.pulsar(ax, ay),
            Some(Accion::Capturar { cursor: true, retardo: 3, guardar: false, .. })
        ));
    }

    #[test]
    fn la_capa_abre_con_las_opciones_recordadas() {
        let mut c = Captura::new(PANTALLA).con_opciones(true, 5);
        let (ax, ay) = centro_accion(&c);
        assert_eq!(c.pulsar(ax, ay), None, "sin marcar, copiar no hace nada");
        c.pulsar(100.0, 100.0);
        c.puntero(300.0, 300.0);
        c.soltar();
        assert!(matches!(
            c.pulsar(ax, ay),
            Some(Accion::Capturar { cursor: true, retardo: 5, .. })
        ));
    }

    #[test]
    fn sin_marcar_los_botones_no_capturan() {
        let mut c = Captura::new(PANTALLA);
        let (ax, ay) = centro_accion(&c);
        assert_eq!(c.pulsar(ax, ay), None);
        assert!(c.arrastre.is_none());
    }

    #[test]
    fn pantalla_marca_todo_y_espera() {
        let mut c = Captura::new(PANTALLA);
        let (px, py) = centro_area(&c, 0);
        assert_eq!(c.pulsar(px, py), None, "elegir el modo no captura");
        let r = c.marcado().expect("la pantalla queda marcada");
        assert_eq!((r.w, r.h), PANTALLA);
        assert_eq!(c.origen_pastilla(), None);
        let (ax, ay) = centro_accion(&c);
        assert_eq!(
            c.pulsar(ax, ay),
            Some(Accion::Capturar {
                x: 0,
                y: 0,
                ancho: 1920,
                alto: 1080,
                guardar: false,
                cursor: false,
                retardo: 0,
            })
        );
    }

    #[test]
    fn modo_video_pide_grabacion_y_pill_detiene() {
        let mut c = Captura::new(PANTALLA);
        let (vx, vy) = centro_tipo(&c, 1);
        c.pulsar(vx, vy);
        let (px, py) = centro_area(&c, 0);
        c.pulsar(px, py);
        let (ax, ay) = centro_accion(&c);
        assert!(matches!(
            c.pulsar(ax, ay),
            Some(Accion::GrabarIniciar { ancho: 1920, alto: 1080, .. })
        ));

        let mut pill = GrabacionPill::new(PANTALLA, true);
        let (ox, oy) = pill.origen();
        assert_eq!(pill.pulsar(ox + 90.0, oy + 20.0), Some(Accion::GrabarSilenciar(true)));
        assert_eq!(pill.pulsar(ox + 125.0, oy + 20.0), Some(Accion::GrabarPausar(true)));
        assert_eq!(pill.pulsar(ox + 200.0, oy + 20.0), Some(Accion::GrabarDetener));
    }

    #[test]
    fn intro_copia() {
        let mut c = Captura::new(PANTALLA);
        assert!(matches!(
            c.tecla(TeclaPulsada::Intro),
            crate::Tecla::Hacer(Accion::Capturar {
                ancho: 1920,
                alto: 1080,
                guardar: false,
                ..
            })
        ));
    }

    #[test]
    fn la_barra_no_arrastra() {
        let mut c = Captura::new(PANTALLA);
        let x = c.x_barra() + 2.0;
        let y = c.y_barra() + 2.0;
        assert_eq!(c.celda_en(x, y), None);
        assert_eq!(c.pulsar(x, y), None);
        assert!(c.arrastre.is_none());
    }

    #[test]
    fn guardar_bytes_png_escribe_atomico_sin_fichero_parcial() {
        let dir = std::env::temp_dir().join(format!("bookos-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("crear dir");
        let destino = dir.join("prueba.png");
        let rgba = vec![255u8; 4 * 4 * 4];
        let png = png_en_memoria(&rgba, 4, 4).expect("codificar png");
        let ruta = guardar_bytes_png(&png, Some(destino.clone())).expect("guardar atomico");
        assert_eq!(ruta, destino);
        let leido = std::fs::read(&ruta).expect("leer fichero");
        assert_eq!(leido, png);
        let hay_part = std::fs::read_dir(&dir)
            .expect("leer dir")
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().ends_with(".part"));
        assert!(!hay_part, "no debe quedar ningún .part");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cambiar_modo_cancela_arrastre_activo() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(100.0, 100.0);
        c.puntero(400.0, 300.0);
        assert!(c.animando());
        assert!(matches!(
            c.tecla(TeclaPulsada::Derecha),
            crate::Tecla::Consumida
        ));
        assert!(!c.animando());
        assert!(c.arrastre.is_none());

        c.pulsar(100.0, 100.0);
        assert!(c.animando());
        let (x, y) = centro_area(&c, 0);
        c.pulsar(x, y);
        assert!(!c.animando());
        assert!(c.arrastre.is_none());
    }

    #[test]
    fn coordenadas_no_finitas_y_cancelar_arrastre_respetan_seleccion_previa() {
        let c = Captura::new((f32::NAN, 0.0));
        assert_eq!(c.pantalla, (1.0, 1.0));

        let mut c = Captura::new(PANTALLA);
        assert!(!c.puntero(f32::NAN, 100.0));
        assert_eq!(c.forma(f32::INFINITY, 100.0), Forma::Cruz);
        assert_eq!(c.pulsar(100.0, f32::NAN), None);
        assert!(!c.animando());

        c.pulsar(100.0, 100.0);
        c.puntero(300.0, 260.0);
        c.cancelar_arrastre();
        assert!(!c.animando());
        assert_eq!(c.marcado(), None);

        c.pulsar(100.0, 100.0);
        c.puntero(300.0, 260.0);
        c.soltar();
        let original = c.marcado().expect("selección confirmada");
        c.pulsar(200.0, 180.0);
        c.puntero(500.0, 420.0);
        c.cancelar_arrastre();
        assert!(!c.animando());
        assert_eq!(c.marcado(), Some(original));
    }

    #[test]
    fn el_popover_responde_en_todos_los_controles() {
        let mut c = Captura::new(PANTALLA);
        let (ox, oy) = centro_opciones(&c);
        c.pulsar(ox, oy);
        assert!(c.popover_abierto());
        let (px, py) = c.origen_opciones().expect("popover abierto");
        let pill_w4 = (OPCIONES_POPOVER_W - 24.0 - 12.0) / 4.0;
        let pill_w2 = (OPCIONES_POPOVER_W - 24.0 - 6.0) / 2.0;

        // Retardo 10s (índice 3)
        assert_eq!(
            c.pulsar(px + 12.0 + 3.0 * (pill_w4 + 4.0) + pill_w4 / 2.0, py + 12.0 + 36.0),
            Some(Accion::CapturaOpciones {
                cursor: false,
                retardo: 10
            })
        );
        // Mostrar puntero
        assert_eq!(
            c.pulsar(px + 40.0, py + 12.0 + 78.0),
            Some(Accion::CapturaOpciones {
                cursor: true,
                retardo: 10
            })
        );
        // Destino: Guardar en Imágenes (índice 1)
        assert_eq!(
            c.pulsar(px + 12.0 + pill_w2 + 6.0 + pill_w2 / 2.0, py + 12.0 + 142.0),
            None
        );
        assert_eq!(c.destino, Destino::Guardar);

        // Cambiar a Vídeo y elegir audio Sistema (índice 2)
        let (vx, vy) = centro_tipo(&c, 1);
        c.pulsar(vx, vy);
        c.pulsar(ox, oy);
        let (px, py) = c.origen_opciones().expect("popover video abierto");
        c.pulsar(px + 12.0 + 2.0 * (pill_w4 + 4.0) + pill_w4 / 2.0, py + 12.0 + 142.0);
        assert_eq!(c.audio, AudioGrabacion::Sistema);
    }

    #[test]
    fn el_pill_de_grabacion_responde_y_repinta_al_arrastrar() {
        let mut pill = GrabacionPill::new(PANTALLA, true);
        let (ox, oy) = pill.origen();
        // Arrastrar desde el área del reloj
        assert_eq!(pill.pulsar(ox + 30.0, oy + 20.0), None);
        assert!(pill.puntero(ox - 50.0, oy + 40.0));
        assert_ne!(pill.origen(), (ox, oy));
        pill.soltar();

        let (nx, ny) = pill.origen();
        assert_eq!(pill.pulsar(nx + 92.0, ny + 21.0), Some(Accion::GrabarSilenciar(true)));
        assert_eq!(pill.pulsar(nx + 126.0, ny + 21.0), Some(Accion::GrabarPausar(true)));
        assert_eq!(pill.pulsar(nx + 195.0, ny + 21.0), Some(Accion::GrabarDetener));
    }
}
