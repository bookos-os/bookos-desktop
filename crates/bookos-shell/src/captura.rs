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

/// Alto de la barra de modos y su separación del borde de abajo.
const BARRA_ALTO: f32 = 76.0;
const BARRA_MARGEN: f32 = 28.0;
const BARRA_PADDING: f32 = 10.0;
const CELDA_W: f32 = 116.0;
const CELDA_H: f32 = 56.0;
const CELDA_HUECO: f32 = 8.0;
/// Lo que ocupa el divisor entre los dos grupos, con su aire a los lados.
const DIVISOR_ANCHO: f32 = 1.0;
const DIVISOR_AIRE: f32 = 12.0;
const DIVISOR_HUECO: f32 = DIVISOR_ANCHO + DIVISOR_AIRE * 2.0;

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

    // 0.1.0
    fn nombre(self) -> &'static str {
        match self {
            Self::Portapapeles => "Copiar",
            Self::Guardar => "Guardar",
        }
    }

    // 0.1.0
    fn simbolo(self) -> &'static str {
        match self {
            Self::Portapapeles => "⧉",
            Self::Guardar => "⤓",
        }
    }
}

/// Lo que se ajusta de la captura sin ser ni el modo ni el destino. Son botones
/// que se quedan puestos —y se recuerdan— en vez de acciones.
// añadido en la 0.2.0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opcion {
    /// Si el cursor sale en la foto.
    Cursor,
    /// Cuántos segundos esperar antes de capturar, para dar tiempo a abrir un
    /// menú o a quitar la mano del ratón.
    Espera,
}

impl Opcion {
    const TODOS: [Self; 2] = [Self::Cursor, Self::Espera];

    // añadido en la 0.2.0
    fn simbolo(self) -> &'static str {
        match self {
            Self::Cursor => "↖",
            Self::Espera => "◷",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// La pantalla entera, sin recortar.
    Pantalla,
    /// Lo que se marque arrastrando.
    Seleccion,
}

impl Modo {
    const TODOS: [Self; 2] = [Self::Pantalla, Self::Seleccion];

    // 0.1.0
    fn nombre(self) -> &'static str {
        match self {
            Self::Pantalla => "Pantalla",
            Self::Seleccion => "Selección",
        }
    }

    /// Un pictograma dibujado con caracteres, como el selector de proyección:
    /// aquí no hay iconos propios y un `<text>` no necesita fuentes cargadas
    /// para estos, que son de la tabla de formas geométricas.
    // 0.1.0
    fn simbolo(self) -> &'static str {
        match self {
            Self::Pantalla => "▣",
            Self::Seleccion => "⛶",
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

pub struct Captura {
    /// La pantalla, en lógicos.
    pantalla: (f32, f32),
    modo: Modo,
    arrastre: Option<Arrastre>,
    /// Lo último que se marcó y sigue marcado tras soltar.
    hecho: Option<Recuadro>,
    /// Qué celda de la barra tiene el ratón encima.
    señalada: Option<usize>,
    /// Si el cursor sale en la foto.
    // añadido en la 0.2.0
    cursor: bool,
    /// Segundos de espera antes de capturar: 0 es inmediata.
    // añadido en la 0.2.0
    retardo: u8,
}

impl Captura {
    // 0.1.0
    pub fn new(pantalla: (f32, f32)) -> Self {
        Self {
            pantalla,
            modo: Modo::Seleccion,
            arrastre: None,
            hecho: None,
            señalada: None,
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

    /// Lo marcado que se ve: el velo tiene el agujero ahí. Es lo mismo que se
    /// capturaría, así que en modo pantalla es la pantalla entera y el marco
    /// la rodea por dentro: es como se ve que la foto va a ser toda.
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

    /// Esquina de la pastilla de las medidas. Debajo del recuadro si cabe y
    /// dentro si no: en una selección que llega al borde de abajo, fuera se
    /// saldría de la pantalla.
    ///
    /// En modo pantalla no hay pastilla: las medidas son las de la pantalla, y
    /// abajo del todo se montaría encima de la barra.
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

    /// Lo que pone la pastilla. Solo cambia cuando cambian los píxeles
    /// enteros, que es lo que decide si hay que repintarla.
    // 0.1.0
    pub fn medidas(&self) -> Option<(i32, i32)> {
        match self.modo {
            Modo::Pantalla => None,
            Modo::Seleccion => self.marcado().map(|r| (r.w as i32, r.h as i32)),
        }
    }

    /// `y` donde empieza la barra de modos.
    // 0.1.0
    fn y_barra(&self) -> f32 {
        self.pantalla.1 - BARRA_MARGEN - BARRA_ALTO
    }

    /// Cuántas celdas tiene la barra: los modos, los destinos y las opciones.
    // modificado en la 0.2.0 -- el tercer grupo, con el cursor y la espera.
    const CELDAS: usize = Modo::TODOS.len() + Destino::TODOS.len() + Opcion::TODOS.len();
    /// Dónde empieza cada grupo después del primero.
    const DESTINOS: usize = Modo::TODOS.len();
    const OPCIONES: usize = Modo::TODOS.len() + Destino::TODOS.len();

    // 0.1.0
    fn ancho_barra(&self) -> f32 {
        CELDA_W * Self::CELDAS as f32
            + CELDA_HUECO * (Self::CELDAS - 1) as f32
            + DIVISOR_HUECO * 2.0
            + BARRA_PADDING * 2.0
    }

    /// La `x` donde empieza la celda `i`, relativa al borde de la pantalla.
    ///
    /// El divisor va entre los dos grupos, así que a partir de la primera celda
    /// de destino todo se corre lo que ocupa. Se calcula aquí y no en el `view`
    /// para que el hit-test y el dibujo no puedan discrepar, que es el fallo
    /// que ya salió una vez en el buscador.
    // 0.1.0
    fn x_celda(&self, i: usize) -> f32 {
        let base = self.x_barra() + BARRA_PADDING + i as f32 * (CELDA_W + CELDA_HUECO);
        // Un divisor por cada frontera de grupo que se haya cruzado.
        let divisores = usize::from(i >= Self::DESTINOS) + usize::from(i >= Self::OPCIONES);
        base + DIVISOR_HUECO * divisores as f32
    }

    // 0.1.0
    fn x_barra(&self) -> f32 {
        (self.pantalla.0 - self.ancho_barra()) / 2.0
    }

    /// Qué celda de la barra cae en ese punto.
    ///
    /// Se recorren las celdas en vez de dividir por el paso: con el divisor en
    /// medio, la cuenta con un solo paso dejaría de valer a partir del segundo
    /// grupo, y cuatro comparaciones no cuestan nada.
    // 0.1.0
    fn celda_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_barra() + BARRA_PADDING;
        if y < y0 || y > y0 + CELDA_H {
            return None;
        }
        (0..Self::CELDAS).find(|i| {
            let x0 = self.x_celda(*i);
            x >= x0 && x < x0 + CELDA_W
        })
    }

    /// ¿Cae el punto sobre la barra de modos? Ahí el arrastre no empieza: si
    /// no, ir a pulsar «Pantalla» marcaría un recuadro por el camino.
    // 0.1.0
    fn en_la_barra(&self, x: f32, y: f32) -> bool {
        x >= self.x_barra()
            && x <= self.x_barra() + self.ancho_barra()
            && y >= self.y_barra()
            && y <= self.y_barra() + BARRA_ALTO
    }

    /// Mueve el puntero. `true` si hay que repintar **la barra**: lo que
    /// cambie en el recuadro lo dicen [`Self::marcado`] y [`Self::medidas`].
    // 0.1.0
    pub fn puntero(&mut self, x: f32, y: f32) -> bool {
        let señalada = self.celda_en(x, y);
        let cambio_señal = señalada != self.señalada;
        self.señalada = señalada;
        let punto = (x.clamp(0.0, self.pantalla.0), y.clamp(0.0, self.pantalla.1));
        match self.arrastre.as_mut() {
            Some(Arrastre::Nuevo(_, hasta)) | Some(Arrastre::Ajuste { hasta, .. }) => {
                *hasta = punto
            }
            None => {}
        }
        cambio_señal
    }

    /// La forma del cursor en ese punto: lo que haría un clic ahí, o lo que se
    /// está haciendo si hay un arrastre.
    // 0.1.0
    pub fn forma(&self, x: f32, y: f32) -> Forma {
        let agarre = match self.arrastre {
            Some(Arrastre::Nuevo(..)) => return Forma::Cruz,
            Some(Arrastre::Ajuste { agarre, .. }) => Some(agarre),
            None if self.en_la_barra(x, y) => return Forma::Flecha,
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

    /// Lo marcado, si se puede ajustar: en modo pantalla no hay aristas que
    /// coger, la foto es toda.
    // 0.1.0
    fn hecho_ajustable(&self) -> Option<Recuadro> {
        self.hecho.filter(|_| self.modo == Modo::Seleccion)
    }

    /// Empieza un arrastre o pulsa un botón de la barra. `Some` solo al pulsar
    /// «Copiar» o «Guardar» con algo que capturar.
    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.celda_en(x, y) {
            // Las últimas son opciones: se quedan puestas y se recuerdan.
            if i >= Self::OPCIONES {
                match Opcion::TODOS[i - Self::OPCIONES] {
                    Opcion::Cursor => self.cursor = !self.cursor,
                    Opcion::Espera => {
                        let siguiente = crate::config::RETARDOS_CAPTURA
                            .iter()
                            .position(|r| *r == self.retardo)
                            .map_or(0, |p| (p + 1) % crate::config::RETARDOS_CAPTURA.len());
                        self.retardo = crate::config::RETARDOS_CAPTURA[siguiente];
                    }
                }
                return Some(Accion::CapturaOpciones {
                    cursor: self.cursor,
                    retardo: self.retardo,
                });
            }
            // Las primeras celdas son los modos y las de después los destinos.
            if let Some(destino) = Destino::TODOS.get(i.wrapping_sub(Modo::TODOS.len())) {
                return self.accion(*destino);
            }
            // Elegir modo no captura: «Pantalla» deja la pantalla entera
            // marcada, y qué hacer con ella se decide con los botones.
            self.modo = Modo::TODOS[i];
            return None;
        }
        if self.en_la_barra(x, y) {
            return None;
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

    /// Suelta el arrastre. **No captura**: deja el recuadro marcado para
    /// elegir después si va al portapapeles o a un fichero.
    ///
    /// Un clic suelto sobre el velo no deja nada marcado: se entiende como «no
    /// quiero nada de aquí» y la capa sigue abierta para volver a intentarlo.
    // 0.1.0
    pub fn soltar(&mut self) {
        let Some(arrastre) = self.arrastre else {
            return;
        };
        let nuevo = self.marcando().filter(Recuadro::vale);
        self.arrastre = None;
        self.hecho = match arrastre {
            Arrastre::Nuevo(..) => nuevo,
            // Encogerlo por debajo del mínimo no borra lo marcado: se queda
            // como estaba, que perderlo por un tirón de más es peor.
            Arrastre::Ajuste { antes, .. } => nuevo.or(Some(antes)),
        };
    }

    // 0.1.0
    fn accion(&self, destino: Destino) -> Option<Accion> {
        let r = self.recuadro()?;
        Some(Accion::Capturar {
            x: r.x as i32,
            y: r.y as i32,
            ancho: r.w as i32,
            alto: r.h as i32,
            guardar: destino == Destino::Guardar,
            cursor: self.cursor,
            retardo: self.retardo,
        })
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> crate::Tecla {
        use crate::Tecla;
        match tecla {
            TeclaPulsada::Escape => Tecla::Cerrar,
            // Intro copia lo que haya: la pantalla entera si no se ha marcado
            // nada, que es lo que se espera de abrir y darle a Intro. Es el
            // atajo del botón «Copiar»; guardar pide el clic.
            TeclaPulsada::Intro => {
                if self.recuadro().is_none() {
                    self.modo = Modo::Pantalla;
                }
                match self.accion(Destino::Portapapeles) {
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
                self.modo = Modo::TODOS[(i + paso) % Modo::TODOS.len()];
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    /// ¿Hay algo moviéndose? El recuadro se mueve con el ratón, así que sí
    /// mientras se arrastra.
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.arrastre.is_some()
    }

    /// La pastilla con las medidas, sola en su lienzo.
    // 0.1.0
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
        // El mismo fondo y el mismo trato que cualquier tarjeta del shell
        // (`emergente::control::tarjeta`): `card()` opaco y sin borde. Lo
        // único propio es el radio, porque esto es una pastilla.
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

    /// La barra, sola en su lienzo: los modos a la izquierda y a dónde va la
    /// captura a la derecha, separados por un divisor. Dónde va en la pantalla
    /// lo dice [`Self::origen_barra`].
    // 0.1.0
    pub fn vista_barra(&self) -> PanelElement<'_> {
        let mut celdas = row![];
        for i in 0..Self::CELDAS {
            if i > 0 {
                celdas = celdas.push(Space::new().width(Length::Fixed(CELDA_HUECO)));
            }
            // El divisor entre los dos grupos: la misma línea que separa las
            // secciones de las tarjetas del panel.
            if i == Self::DESTINOS || i == Self::OPCIONES {
                celdas = celdas
                    .push(Space::new().width(Length::Fixed(DIVISOR_AIRE - CELDA_HUECO)))
                    .push(
                        container(Space::new())
                            .width(Length::Fixed(DIVISOR_ANCHO))
                            .height(Length::Fixed(CELDA_H * 0.6))
                            .style(|_| iced_widget::container::Style {
                                background: Some(tema::alfa(tema::tinta(), 0.14).into()),
                                ..Default::default()
                            }),
                    )
                    .push(Space::new().width(Length::Fixed(DIVISOR_AIRE)));
            }
            // Los modos se encienden cuando están elegidos. Los destinos son
            // botones: no se quedan encendidos, y se apagan mientras no haya
            // nada marcado que capturar.
            let (simbolo, nombre, activa, apagada) = if i >= Self::OPCIONES {
                let o = Opcion::TODOS[i - Self::OPCIONES];
                match o {
                    Opcion::Cursor => (
                        o.simbolo(),
                        if self.cursor {
                            "Con cursor"
                        } else {
                            "Sin cursor"
                        }
                        .to_string(),
                        self.cursor,
                        false,
                    ),
                    Opcion::Espera => (
                        o.simbolo(),
                        if self.retardo == 0 {
                            "Sin espera".to_string()
                        } else {
                            format!("{} s", self.retardo)
                        },
                        self.retardo > 0,
                        false,
                    ),
                }
            } else {
                match Destino::TODOS.get(i.wrapping_sub(Modo::TODOS.len())) {
                    Some(d) => (
                        d.simbolo(),
                        d.nombre().to_string(),
                        false,
                        self.recuadro().is_none(),
                    ),
                    None => {
                        let m = Modo::TODOS[i];
                        (m.simbolo(), m.nombre().to_string(), m == self.modo, false)
                    }
                }
            };
            let señalada = self.señalada == Some(i) && !apagada;
            celdas = celdas.push(
                container(
                    column![
                        text(simbolo).size(22.0).color(if activa {
                            tema::acento()
                        } else {
                            tema::TEXTO2
                        }),
                        Space::new().height(Length::Fixed(2.0)),
                        text(nombre).size(tema::T_PEQUENO).color(if apagada {
                            tema::TEXTO2
                        } else {
                            tema::texto()
                        }),
                    ]
                    .align_x(Horizontal::Center),
                )
                .width(Length::Fixed(CELDA_W))
                .height(Length::Fixed(CELDA_H))
                // `center(Length::Fill)` **no**, aunque sea lo que usan las
                // tarjetas del panel: pone el ancho y el alto a `Fill` y se
                // lleva por delante los fijos de arriba. Allí no se nota porque
                // la emergente ya viene medida al contenido; aquí el padre es
                // la pantalla entera y las celdas se repartían los 1280 px.
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| iced_widget::container::Style {
                    background: Some(
                        if activa {
                            tema::alfa(tema::acento(), 0.16)
                        } else if señalada {
                            tema::hover()
                        } else {
                            iced_core::Color::TRANSPARENT
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: tema::R_CONTROL.into(),
                        width: if activa { 2.0 } else { 0.0 },
                        color: tema::acento(),
                    },
                    ..Default::default()
                }),
            );
        }

        container(celdas)
            .padding(BARRA_PADDING)
            // Medida a lo suyo y no a `Fill`: el lienzo se redondea hacia
            // arriba a píxel entero, y la barra tiene que medir lo mismo que
            // usa `celda_en` para que el clic caiga donde se ve.
            .width(Length::Fixed(self.ancho_barra()))
            .height(Length::Fixed(BARRA_ALTO))
            .style(|_| iced_widget::container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

/// Guarda una captura en el disco y devuelve dónde quedó.
///
/// En `~/Imágenes/Capturas` —o `$XDG_PICTURES_DIR/Capturas`—, con el nombre
/// `Captura 2026-08-24 a las 10.42.31.png`: la fecha delante para que ordenen
/// solas por nombre, y con puntos en la hora porque los dos puntos no se pueden
/// escribir en un nombre de fichero en todos los sistemas de archivos.
// 0.1.0
pub fn guardar_png(rgba: &[u8], ancho: u32, alto: u32) -> std::io::Result<std::path::PathBuf> {
    let base = carpeta_de_capturas();
    std::fs::create_dir_all(&base)?;
    let ruta = base.join(format!("Captura {}.png", sello_de_tiempo()));
    let fichero = std::fs::File::create(&ruta)?;
    codificar_png(std::io::BufWriter::new(fichero), rgba, ancho, alto)?;
    if let Ok(mut guard) = ULTIMA.lock() {
        *guard = Some(ruta.clone());
    }
    Ok(ruta)
}

/// La misma captura, pero en memoria: es lo que se pone en el portapapeles.
// 0.1.0
pub fn png_en_memoria(rgba: &[u8], ancho: u32, alto: u32) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    codificar_png(std::io::Cursor::new(&mut bytes), rgba, ancho, alto)?;
    Ok(bytes)
}

// 0.1.0
fn codificar_png<W: std::io::Write>(
    destino: W,
    rgba: &[u8],
    ancho: u32,
    alto: u32,
) -> std::io::Result<()> {
    // El mismo `image` que ya usa el shell para los avatares y los fondos: vive
    // aquí y no en el compositor para no añadirle una dependencia que ya está a
    // un crate de distancia.
    image::codecs::png::PngEncoder::new(destino)
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

    /// El centro de la celda `i` de la barra.
    fn celda(c: &Captura, i: usize) -> (f32, f32) {
        (
            c.x_celda(i) + CELDA_W / 2.0,
            c.y_barra() + BARRA_PADDING + CELDA_H / 2.0,
        )
    }

    /// El arrastre se queda dentro de la pantalla: sacar el ratón por el borde
    /// no puede pedir píxeles que no existen.
    #[test]
    fn el_arrastre_no_se_sale_de_la_pantalla() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(100.0, 100.0);
        c.puntero(5000.0, 5000.0);
        let r = c.recuadro().unwrap();
        assert_eq!((r.x + r.w, r.y + r.h), (PANTALLA.0, PANTALLA.1));
    }

    /// Cada celda de la barra responde en su sitio, con el divisor en medio: si
    /// el dibujo y el hit-test se separan, se pulsa una y se enciende otra.
    #[test]
    fn cada_celda_de_la_barra_responde_en_su_sitio() {
        let c = Captura::new(PANTALLA);
        let y = c.y_barra() + BARRA_PADDING + 4.0;
        for i in 0..Captura::CELDAS {
            assert_eq!(c.celda_en(c.x_celda(i) + 2.0, y), Some(i), "la celda {i}");
            assert_eq!(
                c.celda_en(c.x_celda(i) + CELDA_W - 1.0, y),
                Some(i),
                "el borde derecho de la celda {i}"
            );
            // Y el hueco que hay justo después no es de nadie.
            assert_eq!(c.celda_en(c.x_celda(i) + CELDA_W + 2.0, y), None);
        }
        // La barra entera cabe en la pantalla y va centrada.
        assert!(c.x_barra() > 0.0);
        assert!(c.x_barra() + c.ancho_barra() < PANTALLA.0);
    }

    /// Con algo marcado, «Copiar» y «Guardar» capturan eso mismo, cada uno a
    /// su sitio.
    #[test]
    fn copiar_y_guardar_capturan_lo_marcado() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        c.soltar();
        let esperada = |guardar| {
            Some(Accion::Capturar {
                x: 200,
                y: 150,
                ancho: 400,
                alto: 300,
                guardar,
                cursor: false,
                retardo: 0,
            })
        };
        let copiar = celda(&c, Modo::TODOS.len());
        let guardar = celda(&c, Modo::TODOS.len() + 1);
        assert_eq!(c.pulsar(copiar.0, copiar.1), esperada(false));
        assert_eq!(c.pulsar(guardar.0, guardar.1), esperada(true));
    }

    /// El cursor sale fuera por defecto y las opciones viajan con la captura.
    // añadido en la 0.2.0
    #[test]
    fn el_cursor_y_la_espera_viajan_con_la_captura() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        c.soltar();
        let cursor = celda(&c, Captura::OPCIONES);
        let espera = celda(&c, Captura::OPCIONES + 1);
        // Pulsar una opción no captura: pide guardarla y se queda puesta.
        assert_eq!(
            c.pulsar(cursor.0, cursor.1),
            Some(Accion::CapturaOpciones {
                cursor: true,
                retardo: 0
            })
        );
        assert_eq!(
            c.pulsar(espera.0, espera.1),
            Some(Accion::CapturaOpciones {
                cursor: true,
                retardo: 3
            })
        );
        let copiar = celda(&c, Modo::TODOS.len());
        assert!(matches!(
            c.pulsar(copiar.0, copiar.1),
            Some(Accion::Capturar {
                cursor: true,
                retardo: 3,
                guardar: false,
                ..
            })
        ));
        // La espera da la vuelta: 3, 5, 10 y otra vez sin espera.
        for esperado in [5, 10, 0] {
            let Some(Accion::CapturaOpciones { retardo, .. }) = c.pulsar(espera.0, espera.1) else {
                panic!("la espera es una opción");
            };
            assert_eq!(retardo, esperado);
        }
    }

    /// Con las opciones recordadas, la capa abre como se dejó.
    // añadido en la 0.2.0
    #[test]
    fn la_capa_abre_con_las_opciones_recordadas() {
        let mut c = Captura::new(PANTALLA).con_opciones(true, 5);
        let (x, y) = celda(&c, Modo::TODOS.len());
        assert_eq!(c.pulsar(x, y), None, "sin marcar, copiar no hace nada");
        c.pulsar(100.0, 100.0);
        c.puntero(300.0, 300.0);
        c.soltar();
        assert!(matches!(
            c.pulsar(x, y),
            Some(Accion::Capturar {
                cursor: true,
                retardo: 5,
                ..
            })
        ));
    }

    /// Sin nada marcado los botones no hacen nada: no hay foto que decidir.
    #[test]
    fn sin_marcar_los_botones_no_capturan() {
        let mut c = Captura::new(PANTALLA);
        let (x, y) = celda(&c, Modo::TODOS.len() + 1);
        assert_eq!(c.pulsar(x, y), None);
        assert!(
            c.arrastre.is_none(),
            "pulsar un botón no empieza un arrastre"
        );
    }

    /// «Pantalla» marca la pantalla entera y espera a que se elija qué hacer.
    #[test]
    fn pantalla_marca_todo_y_espera() {
        let mut c = Captura::new(PANTALLA);
        let (x, y) = celda(&c, 0);
        assert_eq!(c.pulsar(x, y), None, "elegir el modo no captura");
        let r = c.marcado().expect("la pantalla queda marcada");
        assert_eq!((r.w, r.h), PANTALLA);
        assert_eq!(
            c.origen_pastilla(),
            None,
            "en pantalla entera no hay pastilla"
        );
        let (x, y) = celda(&c, Modo::TODOS.len() + 1);
        assert_eq!(
            c.pulsar(x, y),
            Some(Accion::Capturar {
                x: 0,
                y: 0,
                ancho: 1920,
                alto: 1080,
                guardar: true,
                cursor: false,
                retardo: 0,
            })
        );
    }

    /// Intro copia sin más: lo marcado, o la pantalla entera si no hay nada.
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

    /// Y la barra no marca recuadro: ir a pulsar un botón no puede dejar una
    /// selección hecha por el camino.
    #[test]
    fn la_barra_no_arrastra() {
        let mut c = Captura::new(PANTALLA);
        // Un punto de la tarjeta que no cae en ninguna celda: el relleno.
        let x = c.x_barra() + 2.0;
        let y = c.y_barra() + 2.0;
        assert_eq!(c.celda_en(x, y), None);
        assert_eq!(c.pulsar(x, y), None);
        assert!(
            c.arrastre.is_none(),
            "la barra no puede empezar un arrastre"
        );
    }
}
