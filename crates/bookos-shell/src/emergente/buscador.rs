//! El buscador: Meta+Espacio, escribe y pasa algo.
//!
//! Es el hueco que dejaba KRunner al salir de Plasma. Busca tres cosas, y las
//! tres salen de sitios que el shell ya sabe leer:
//!
//! - **Aplicaciones**, de los `.desktop` ([`crate::apps`]), con la misma
//!   puntuación que el launchpad: escribir «fir» tiene que poner Firefox el
//!   primero en los dos sitios o el escritorio se siente cosido a mano.
//! - **Comandos**: lo que empiece por `>` se ejecuta tal cual, y lo que sea el
//!   nombre de un ejecutable del `PATH` se ofrece como comando aunque además
//!   haya aplicaciones que encajen.
//! - **Estados**: «batería», «red», «brillo», «hora», «memoria». Se responden
//!   ahí mismo en vez de abrir la tarjeta del panel, que es lo que hace que
//!   preguntar sea más rápido que ir a mirarlo.
//!
//! ## Ámbitos: Zenith y archivos
//!
//! Como las acciones de Spotlight en macOS 26: escribir el nombre de quien
//! tiene que atender y un espacio (o Tab sobre su sugerencia) lo convierte en
//! una píldora delante del texto, y desde ahí lo escrito es para él. A la
//! derecha va la app que hará lo elegido: «Zenith · busca un archivo llamado
//! BookOS-HIG · Explorador». Retroceso con el campo vacío quita la píldora.
//!
//! Los archivos salen del índice de `plocate`, en un hilo: filtrar la carpeta
//! personal sin lo oculto tarda 120-130 ms (medido con «hig» y con una
//! palabra que no está), y en el hilo del shell eso son ocho fotogramas
//! congelados por tecla. Mientras llegan, `animando()` mantiene el repintado
//! y `recoger()` los mete al pintar.
//!
//! ## Por qué no hay un catálogo de «runners»
//!
//! KRunner tiene plugins porque los carga de fuera. Aquí las tres fuentes están
//! compiladas dentro y se conocen todas: un trait con tres implementaciones
//! fijas sería indirección sin nadie a quien servir. Cuando haya una cuarta que
//! venga de fuera del binario, ese será el momento.
//!
//! ## Lo que no hace
//!
//! No guarda historial ni aprende de lo que eliges. Ordenar por uso reciente
//! obliga a escribir en disco en cada pulsación de Intro, y sin medir si
//! molesta no se paga.

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, stack, text};

use crate::icono::{self, COLOR_TEMAS, Icono, insignia};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, PanelData, apps};

use super::control::{self, ENTRE_BARRA, ICONO_BARRA};
use super::{Ancla, FilaAccesible, Tecla, t};

/// Ancho de referencia del diálogo. El valor real se adapta a la pantalla
/// lógica: en un monitor grande no se queda como una miniatura centrada y en
/// una salida pequeña no se sale por los lados.
///
/// Del tamaño de Spotlight: con 640 y texto de 16 se leía como un campo de
/// formulario, no como el sitio desde el que se hace todo.
pub const ANCHO: f32 = 720.0;
const ANCHO_MIN: f32 = 600.0;
const ANCHO_MAX: f32 = 820.0;
/// Alto del campo de texto.
const CAMPO: f32 = 68.0;
/// Cuerpo de lo que se escribe.
const TEXTO_CAMPO: f32 = 22.0;
/// Alto de una fila de resultado.
const FILA: f32 = 54.0;
/// Márgenes internos.
const MARGEN: f32 = 10.0;
/// Línea y aire entre el campo y los resultados.
const DIVISOR: f32 = 1.0;
const SEPARACION: f32 = 8.0;
/// Cuántos resultados se enseñan. Más de seis y la lista pide desplazarse, que
/// es justo lo que un buscador de teclado no quiere.
const MAXIMO: usize = 6;
/// Lado del icono de una fila.
const ICONO: f32 = 30.0;
/// Lo que se abre bajo la fila elegida de brillo o volumen: la barra de las
/// tarjetas (`control::FILA_BARRA`) y 10 de aire. Medido sobre el diseño:
/// 54 de fila + 28 de barra + 10 = 92.
// añadido en la 0.2.2
const BARRA_EXTRA: f32 = control::FILA_BARRA + 10.0;
/// Dónde empieza el texto de una fila, y su margen derecho: de ahí salen el
/// primer y último punto de la barra.
// añadido en la 0.2.2
const X_TEXTO: f32 = MARGEN + (18.0 - MARGEN) + ICONO + 14.0;
const DERECHA_FILA: f32 = MARGEN + 14.0;
/// Cuántas aplicaciones se sugieren con el campo vacío.
const SUGERENCIAS: usize = 4;

#[derive(Clone, Copy)]
struct Colores {
    tarjeta: Color,
    texto: Color,
    secundario: Color,
    divisor: Color,
    borde: Color,
    hover: Color,
    acento: Color,
}

impl Colores {
    // 0.1.0
    fn actuales() -> Self {
        let respaldo = Self {
            tarjeta: tema::card(),
            texto: tema::texto(),
            secundario: tema::texto2(),
            divisor: tema::divisor(),
            borde: tema::borde(),
            hover: tema::hover(),
            acento: tema::acento(),
        };
        if tema::alto_contraste() {
            return respaldo;
        }
        // Ajustes publica el JSON para renderizadores sin CSS. Se relee al
        // dibujar para que reabrir o escribir no conserve una semilla antigua.
        let paleta = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .and_then(|dir| std::fs::read_to_string(dir.join(".config/bookos/palette.json")).ok())
            .and_then(|json| Self::desde_json(&json, tema::es_claro()));
        paleta.unwrap_or(respaldo)
    }

    // 0.1.0
    fn desde_json(json: &str, claro: bool) -> Option<Self> {
        let paleta: serde_json::Value = serde_json::from_str(json).ok()?;
        let roles = paleta.get(if claro { "light" } else { "dark" })?;
        let rol = |nombre| tema::color_css(roles.get(nombre)?.as_str()?);
        Some(Self {
            tarjeta: rol("card")?,
            texto: rol("tx")?,
            secundario: rol("tx2")?,
            divisor: rol("div")?,
            borde: rol("brd")?,
            hover: rol("hov")?,
            acento: rol("blue")?,
        })
    }
}

/// A quién va lo escrito. Ver «Ámbitos» arriba.
// añadido en la 0.2.2
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ambito {
    Todo,
    Zenith,
    Archivos,
}

impl Ambito {
    // añadido en la 0.2.2
    fn nombre(self) -> &'static str {
        match self {
            Self::Todo => "",
            Self::Zenith => "Zenith",
            Self::Archivos => t("Archivos", "Files"),
        }
    }

    /// La palabra que, escrita delante, abre el ámbito.
    // añadido en la 0.2.2
    fn por_palabra(palabra: &str) -> Option<Self> {
        // modificado en la 0.2.2 -- también con @ delante, como las menciones de Raycast
        match palabra.trim_start_matches('@').to_lowercase().as_str() {
            "zenith" => Some(Self::Zenith),
            "archivos" | "archivo" | "explorador" | "files" => Some(Self::Archivos),
            _ => None,
        }
    }
}

/// Qué ajuste lleva la barra de una fila de nivel.
// añadido en la 0.2.2
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Que {
    Brillo,
    Volumen,
}

impl Que {
    /// Brillo no baja de 5: a cero no se ve ni la pantalla con la que volver
    /// a subirlo (el mismo suelo que el deslizador y las teclas).
    fn minimo(self) -> u8 {
        match self {
            Self::Brillo => 5,
            Self::Volumen => 0,
        }
    }

    fn accion(self, nivel: u8) -> Accion {
        match self {
            Self::Brillo => Accion::Brillo(nivel),
            Self::Volumen => Accion::Volumen(nivel),
        }
    }
}

/// Lo que se puede encontrar.
pub enum Resultado {
    /// Una aplicación instalada.
    App {
        nombre: String,
        detalle: Option<String>,
        exec: String,
        icono: Option<Icono>,
        /// El `app_id` de su ventana si ya está abierta: entonces elegirla te
        /// lleva a ella en vez de abrir otra.
        abierta: Option<String>,
    },
    /// Una orden para el intérprete.
    Comando(String),
    /// Acción del propio escritorio. Se presenta igual que un comando, pero
    /// el compositor puede abrir una tarjeta o bloquear la sesión sin pasar
    /// por un shell.
    Accion {
        nombre: String,
        detalle: String,
        icono: Option<Icono>,
        accion: Accion,
    },
    /// Un archivo o carpeta del índice de `plocate`. Se abre en el
    /// Explorador, que selecciona el archivo dentro de su carpeta.
    // añadido en la 0.2.2
    Archivo {
        ruta: std::path::PathBuf,
        icono: Option<Icono>,
    },
    /// Lo que contesta Zenith en el propio buscador: dónde está lo que se le
    /// pidió. Intro copia la ruta, que es lo que se quiere al preguntar
    /// «dónde está»; abrirlo es la fila de debajo, el mismo archivo.
    // añadido en la 0.2.2
    Respuesta {
        titulo: String,
        ruta: std::path::PathBuf,
        icono: Option<Icono>,
    },
    /// Llevar lo escrito a otro ámbito. Intro sobre Zenith pregunta ya; Tab
    /// (o Intro sobre Archivos) entra en el ámbito con el texto puesto.
    // añadido en la 0.2.2
    Ambito {
        ambito: Ambito,
        texto: String,
        icono: Option<Icono>,
    },
    /// «@» a secas o con el principio de un nombre: a quién se le puede
    /// hablar. Intro, Tab o un clic ponen su píldora.
    // añadido en la 0.2.2
    Mencion {
        ambito: Ambito,
        detalle: &'static str,
        icono: Option<Icono>,
    },
    /// Brillo o volumen con su barra: la elegida se abre hacia abajo. Lo que
    /// se mueve queda pendiente en el buscador y Intro lo aplica.
    // añadido en la 0.2.2
    Nivel {
        que: Que,
        valor: u8,
        mudo: bool,
        icono: Option<Icono>,
        /// El dibujo de los extremos de la barra, sin la insignia de color.
        glifo: Option<Icono>,
    },
    /// Una respuesta que se lee aquí mismo; si `copiar` trae texto, Intro la
    /// copia al portapapeles.
    Dato {
        titulo: String,
        valor: String,
        copiar: Option<String>,
        icono: Option<Icono>,
    },
}

impl Resultado {
    // 0.1.0
    fn nombre(&self) -> &str {
        match self {
            Self::App { nombre, .. } => nombre,
            Self::Comando(orden) => orden,
            Self::Accion { nombre, .. } => nombre,
            Self::Dato { titulo, .. } | Self::Respuesta { titulo, .. } => titulo,
            Self::Archivo { ruta, .. } => ruta
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default(),
            Self::Ambito { ambito, .. } => match ambito {
                Ambito::Zenith => t("Preguntar a Zenith", "Ask Zenith"),
                Ambito::Archivos | Ambito::Todo => t("Buscar en archivos", "Search files"),
            },
            Self::Mencion { ambito, .. } => match ambito {
                Ambito::Zenith => "@zenith",
                Ambito::Archivos | Ambito::Todo => t("@archivos", "@files"),
            },
            Self::Nivel { que, .. } => match que {
                Que::Brillo => t("Brillo", "Brightness"),
                Que::Volumen => t("Volumen", "Volume"),
            },
        }
    }

    /// La app que hará lo elegido, para la píldora de la derecha. Las apps no
    /// la llevan: su nombre ya es la fila.
    // añadido en la 0.2.2
    fn destino(&self) -> Option<&'static str> {
        match self {
            Self::Archivo { .. } => Some(t("Explorador", "Explorer")),
            _ => None,
        }
    }

    /// ¿Acepta confirmación con `Intro` (lanzar, entrar o copiar)?
    fn tiene_intro(&self) -> bool {
        match self {
            Self::Dato { copiar, .. } => copiar.is_some(),
            _ => true,
        }
    }

    /// El renglón pequeño de debajo: qué es esto y qué va a pasar al pulsarlo.
    // 0.1.0
    // modified 0.1.1 -- las abiertas dicen «Abierta · ir a la ventana»
    fn detalle(&self) -> String {
        match self {
            Self::App {
                abierta: Some(_), ..
            } => t("Abierta · ir a la ventana", "Open · switch to window").to_string(),
            Self::App {
                detalle: Some(desc),
                ..
            } => desc.clone(),
            Self::App { detalle: None, .. } => t("Aplicación", "Application").to_string(),
            Self::Comando(_) => t("Ejecutar comando", "Run command").to_string(),
            Self::Accion { detalle, .. } => detalle.clone(),
            Self::Dato { valor, .. } => valor.clone(),
            Self::Archivo { ruta, .. } => ruta.parent().map(con_tilde).unwrap_or_default(),
            Self::Respuesta { ruta, .. } => format!(
                "{} · {}",
                con_tilde(ruta),
                t("Intro copia la ruta", "Enter copies the path")
            ),
            Self::Ambito { texto, .. } => format!(
                "«{texto}» · {}",
                t("Tab para seguir aquí", "Tab to continue here")
            ),
            Self::Mencion { detalle, .. } => (*detalle).to_string(),
            Self::Nivel {
                que: Que::Brillo,
                valor,
                ..
            } => format!("{} · {valor} %", t("Pantalla integrada", "Built-in display")),
            Self::Nivel {
                que: Que::Volumen,
                valor,
                mudo,
                ..
            } => format!(
                "{} · {}",
                t("Altavoces", "Speakers"),
                if *mudo {
                    t("Silenciado", "Muted").to_string()
                } else {
                    format!("{valor} %")
                }
            ),
        }
    }

    // 0.1.0
    fn icono(&self) -> Option<&Icono> {
        match self {
            Self::App { icono, .. } | Self::Dato { icono, .. } => icono.as_ref(),
            Self::Comando(_) => None,
            Self::Accion { icono, .. }
            | Self::Archivo { icono, .. }
            | Self::Respuesta { icono, .. }
            | Self::Mencion { icono, .. }
            | Self::Nivel { icono, .. }
            | Self::Ambito { icono, .. } => icono.as_ref(),
        }
    }

    /// Qué hacer al elegirlo. Un dato con valor copiable lo lleva al portapapeles.
    // 0.1.0
    // modified 0.1.1 -- una app abierta lleva a su ventana en vez de abrir otra
    fn accion(&self) -> Option<Accion> {
        match self {
            Self::App {
                exec,
                abierta: Some(app_id),
                ..
            } => Some(Accion::Mostrar {
                app_id: app_id.clone(),
                exec: exec.clone(),
            }),
            Self::App { exec, .. } => Some(Accion::Lanzar(exec.clone())),
            Self::Comando(orden) => Some(Accion::Lanzar(orden.clone())),
            Self::Accion { accion, .. } => Some(accion.clone()),
            Self::Dato { copiar, .. } => copiar.clone().map(Accion::CopiarTexto),
            Self::Respuesta { ruta, .. } => {
                Some(Accion::CopiarTexto(ruta.to_string_lossy().into_owned()))
            }
            Self::Archivo { ruta, .. } => Some(Accion::Lanzar(format!(
                "bookos-explorer {}",
                crate::escritorio::entrecomillar(ruta)
            ))),
            Self::Ambito {
                ambito: Ambito::Zenith,
                texto,
                ..
            } => Some(preguntar_a_zenith(texto)),
            Self::Ambito { .. } | Self::Mencion { .. } => None,
            // Lo decide el buscador, que es quien guarda lo pendiente.
            Self::Nivel { .. } => None,
        }
    }
}

/// `bookos-assistant --preguntar` se lo pasa por su socket a la instancia que
/// ya corre, que lo trata como una orden dicha: misma isla, mismas acciones.
// añadido en la 0.2.2
fn preguntar_a_zenith(texto: &str) -> Accion {
    Accion::Lanzar(format!(
        "bookos-assistant --preguntar {}",
        crate::escritorio::entrecomillar(std::path::Path::new(texto))
    ))
}

/// La ruta con `~` en vez de la carpeta personal: es como se lee y se dice.
// añadido en la 0.2.2
fn con_tilde(ruta: &std::path::Path) -> String {
    let texto = ruta.to_string_lossy();
    match std::env::var("HOME") {
        Ok(casa) if texto.starts_with(&casa) => format!("~{}", &texto[casa.len()..]),
        _ => texto.into_owned(),
    }
}

fn icono_zenith() -> Option<Icono> {
    icono::cargar("bookos-assistant").or_else(|| insignia("micro"))
}

pub struct Buscador {
    /// Todo el layout se expresa en píxeles lógicos. El compositor rasteriza
    /// después a físicos con la escala del monitor, evitando que HiDPI cambie
    /// el tamaño aparente o desplace el hit-test.
    ancho: f32,
    consulta: String,
    /// Posición del cursor en bytes dentro de `consulta` (siempre frontera UTF-8).
    cursor: usize,
    // añadido en la 0.2.2
    ambito: Ambito,
    /// Búsqueda de archivos pendiente de debounce (evita lanzar un `plocate`
    /// por cada tecla cuando se escribe rápido).
    pendiente_archivos: Option<(String, std::time::Instant)>,
    /// La búsqueda de archivos en marcha. Una nueva sustituye a la anterior:
    /// soltar el receptor basta para que su resultado se tire al llegar.
    // añadido en la 0.2.2
    en_curso: Option<(String, std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>)>,
    /// Las aplicaciones, leídas una vez al abrir: igual que el launchpad, que
    /// no vigila el directorio con inotify por un `dnf install` al mes.
    apps: Vec<apps::App>,
    /// Lanzamientos por `exec`, los mismos que ordenan el launchpad: entre dos
    /// que encajan igual de bien va primero la que usas.
    usos: std::collections::HashMap<String, u32>,
    /// `(exec, app_id)` de lo que tiene ventana, sacado del dock al abrir.
    abiertas: Vec<(String, String)>,
    resultados: Vec<Resultado>,
    /// Cuál está elegido. Siempre hay uno mientras haya resultados: el buscador
    /// se usa con Intro, y sin selección Intro no haría nada.
    seleccion: usize,
    /// Lo señalado con el ratón, que es otra cosa que lo elegido con el
    /// teclado: mover el ratón por encima no debe cambiar lo que Intro lanza.
    // modificado en la 0.2.0 -- con realce que se funde, como las listas de las tarjetas
    señalada: tema::Realce,
    /// Dónde está el fondo de la elegida, en filas. Es uno solo que se
    /// desliza: apagar una fila y encender otra no decía hacia dónde ibas.
    // añadido en la 0.2.0
    elegida_y: tema::Transicion,
    /// El alto de la tarjeta. El buffer se reserva al máximo y la tarjeta se
    /// recorta dentro: rehacer la superficie en cada fotograma del crecimiento
    /// costaría una textura nueva por fotograma.
    // añadido en la 0.2.0
    alto: tema::Transicion,
    /// Tras Ctrl+A: lo siguiente que se escriba o borre sustituye la consulta
    /// entera.
    todo_seleccionado: bool,
    /// Lo movido en la barra de brillo o volumen y aún sin aplicar: Intro lo
    /// aplica. Aplicar al arrastrar dejaba la pantalla a oscuras a medio gesto.
    // añadido en la 0.2.2
    pendiente: Option<(Que, u8)>,
    /// La barra está agarrada con el ratón.
    // añadido en la 0.2.2
    agarrada: bool,
    /// Lo último aplicado desde la barra o copiado: cubre la línea de la fila
    /// con el sello ✓ y lo que quedó, y al vencer cierra el buscador.
    // añadido en la 0.2.2
    confirmando: Option<(std::time::Instant, String)>,
    /// El sello crece de 0,4 a 1 al aparecer.
    sello: tema::Transicion,
}

/// Cuánto se queda la confirmación antes de cerrar: lo que se tarda en leer
/// «Brillo al 30 %», sin hacer esperar a quien ya ha terminado.
// añadido en la 0.2.2
const CONFIRMACION: std::time::Duration = std::time::Duration::from_millis(1200);
/// Espera tras la última pulsación antes de lanzar `plocate` en segundo plano.
const DEBOUNCE_ARCHIVOS: std::time::Duration = std::time::Duration::from_millis(65);

impl Buscador {
    // 0.1.0
    // modified 0.1.1 -- conoce las abiertas y los usos, y olvida iconos que no estaban
    pub fn new(pantalla: (f32, f32), abiertas: Vec<(String, String)>) -> Self {
        crate::icono::olvidar_ausentes();
        let apps = apps::leer();
        let mut buscador = Self {
            ancho: ancho_para(pantalla.0),
            consulta: String::new(),
            cursor: 0,
            ambito: Ambito::Todo,
            pendiente_archivos: None,
            en_curso: None,
            apps,
            usos: crate::config::cargar_usos(),
            abiertas,
            resultados: Vec::new(),
            seleccion: 0,
            señalada: tema::Realce::nuevo(),
            elegida_y: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            alto: tema::Transicion::nueva(CAMPO, tema::M_RAPIDO),
            todo_seleccionado: false,
            pendiente: None,
            agarrada: false,
            confirmando: None,
            sello: tema::Transicion::nueva(1.0, tema::M_REBOTE),
        };
        buscador.buscar();
        // Al abrir ya tiene su alto: la tarjeta entra entera, no creciendo.
        let alto = buscador.alto_objetivo();
        buscador.alto.fijar(alto);
        buscador
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// Relee los `.desktop` y vuelve a resolver los iconos tras cambiar de tema.
    pub fn recargar_iconos(&mut self) {
        self.apps = apps::leer();
        self.buscar();
    }

    // 0.1.0
    // modified 0.2.0 -- el alto máximo: lo visible lo da `alto_visible`
    pub fn size(&self) -> (f32, f32) {
        // La barra de una fila de nivel pide sitio de más solo mientras la hay: así
        // el buffer de las demás búsquedas sigue siendo el de seis filas justas.
        let barra = self
            .resultados
            .iter()
            .any(|r| matches!(r, Resultado::Nivel { .. }));
        (
            self.ancho,
            Self::alto_para(MAXIMO) + if barra { BARRA_EXTRA } else { 0.0 },
        )
    }

    /// Lo que ocupa la tarjeta ahora, dentro del buffer. Por debajo es
    /// transparente y un clic ahí tiene que contar como fuera.
    // añadido en la 0.2.0
    pub fn alto_visible(&self) -> f32 {
        self.alto.valor()
    }

    /// Lo que mide la tarjeta con esa cantidad de resultados.
    ///
    /// Tiene que cuadrar con lo que apila `view()`: campo, divisor, las filas y
    /// el respiro de abajo.
    ///
    /// Antes contaba `+ MARGEN` sin que ese margen existiera en la vista: el
    /// buffer salía nueve píxeles más alto que la tarjeta y esos nueve píxeles
    /// eran **transparentes**, no fondo. O sea que la tarjeta acababa nueve
    /// píxeles antes del borde del buffer, y como el compositor centra por el
    /// buffer, se dibujaba cuatro píxeles y medio por encima de donde tocaba.
    /// El respiro sí se quiere —pegada al borde, la última fila se comía su
    /// esquina redondeada—, así que ahora está dentro de la tarjeta y contado.
    // 0.1.0
    fn alto_para(resultados: usize) -> f32 {
        let filas = resultados.min(MAXIMO) as f32;
        // Sin resultados no hay lista: la tarjeta es solo el campo, y así el
        // buscador vacío no es un rectángulo con un agujero.
        if filas == 0.0 {
            CAMPO
        } else {
            CAMPO + DIVISOR + SEPARACION + filas * FILA + MARGEN
        }
    }

    /// Alto que usa el compositor para colocar el buscador.
    ///
    /// Es estable para que el campo no salte al aparecer resultados, pero es
    /// el alto del **campo**, no el máximo de toda la lista: así lo que se
    /// centra al abrir Spotlight es aquello con lo que se interactúa. Los
    /// resultados crecen hacia abajo sin desplazarlo.
    // 0.1.0
    pub fn alto_colocacion() -> f32 {
        CAMPO
    }

    /// Sin velo, como Spotlight: el buscador es algo que se consulta de paso,
    /// y oscurecer la pantalla lo convertía en un modal que tapa lo que estás
    /// haciendo —que a menudo es justo de donde sale lo que vas a escribir—.
    // 0.1.0
    // modified 0.1.1 -- sin velo oscuro detrás
    pub fn velo(&self) -> iced_core::Color {
        iced_core::Color::TRANSPARENT
    }

    /// Rehace la lista con lo que hay escrito.
    // 0.1.0
    // modified 0.1.1 -- ordena con usos y abiertas
    fn buscar(&mut self) {
        self.cursor = self.cursor.min(self.consulta.len());
        // «@» sin espacio aún: es la lista de a quién se le puede hablar, no una búsqueda.
        let mencion = (self.ambito == Ambito::Todo)
            .then(|| self.consulta.strip_prefix('@'))
            .flatten()
            .filter(|resto| !resto.contains(char::is_whitespace))
            .map(menciones);
        self.resultados = match self.ambito {
            _ if mencion.is_some() => mencion.unwrap_or_default(),
            Ambito::Todo => {
                let mut lista = resultados(&self.apps, &self.usos, &self.abiertas, &self.consulta);
                lista.extend(puertas(&self.consulta, lista.len()));
                if let Some(nivel) = self.nivel_para() {
                    // El atajo que abre la tarjeta del mismo nombre sobra: la
                    // fila de nivel ya hace más.
                    let propio = apps::normalizar(nivel.nombre());
                    lista.retain(|r| {
                        !matches!(r, Resultado::Accion { nombre, .. } if apps::normalizar(nombre) == propio)
                    });
                    lista.insert(0, nivel);
                    lista.truncate(MAXIMO);
                }
                lista
            }
            Ambito::Zenith => en_zenith(&self.consulta),
            Ambito::Archivos => Vec::new(),
        };
        let nombre = match self.ambito {
            Ambito::Todo => None,
            Ambito::Zenith => archivo_pedido(self.consulta.trim()).map(str::to_string),
            Ambito::Archivos => Some(self.consulta.trim().to_string()),
        };
        self.en_curso = None;
        self.pendiente_archivos = nombre.filter(|n| n.chars().count() >= 2).map(|nombre| {
            // Los archivos usados hace poco aparecen al momento, sin esperar a `plocate`.
            let recientes = buscar_recientes(&nombre);
            if !recientes.is_empty() {
                let mut nuevos = Vec::new();
                if self.ambito == Ambito::Zenith
                    && let Some(ruta) = recientes.first()
                {
                    nuevos.push(respuesta_zenith(ruta));
                }
                let hueco = MAXIMO
                    .saturating_sub(nuevos.len())
                    .saturating_sub(usize::from(self.ambito == Ambito::Zenith));
                nuevos.extend(recientes.into_iter().take(hueco).map(resultado_archivo));
                self.resultados.splice(0..0, nuevos);
                self.resultados.truncate(MAXIMO);
            }
            (nombre, std::time::Instant::now())
        });
        self.seleccion = 0;
        self.pendiente = None;
        self.agarrada = false;
        self.confirmando = None;
        // Las filas ya son otras: ni el realce ni la elegida vienen de la
        // anterior, así que no se deslizan desde ella.
        self.señalada = tema::Realce::nuevo();
        self.elegida_y.fijar(0.0);
        self.alto.ir_a(self.alto_objetivo());
    }

    /// La fila de brillo o volumen con su barra, si lo escrito es el principio
    /// de su nombre («bri», «vol»…). Con menos de tres letras encajaría media
    /// lista. Si no se puede leer el valor, no hay fila: una barra a cero
    /// mentiría.
    // añadido en la 0.2.2
    fn nivel_para(&self) -> Option<Resultado> {
        let q = apps::normalizar(self.consulta.trim());
        if q.chars().count() < 3 || q.contains(char::is_whitespace) {
            return None;
        }
        let empieza = |palabras: &[&str]| palabras.iter().any(|p| p.starts_with(&q));
        if empieza(&["brillo", "pantalla", "brightness"]) {
            let valor = PanelData::read().brightness?;
            return Some(Resultado::Nivel {
                que: Que::Brillo,
                valor,
                mudo: false,
                icono: insignia("brillo"),
                glifo: icono::propio("brillo"),
            });
        }
        if empieza(&["volumen", "sonido", "audio", "altavoces", "volume"]) {
            let (valor, mudo) = crate::widgets::volumen::consultar("@DEFAULT_AUDIO_SINK@")?;
            return Some(Resultado::Nivel {
                que: Que::Volumen,
                valor,
                mudo,
                icono: insignia("volumen-alto"),
                glifo: icono::propio("volumen-alto"),
            });
        }
        None
    }

    /// Dónde empieza la lista dentro de la tarjeta. Tiene que incluir todo el
    /// aire que apila `view()` entre el campo y los resultados.
    // 0.1.0
    fn y_lista(&self) -> f32 {
        CAMPO + DIVISOR + SEPARACION
    }

    /// Lo que mide la fila `i`: la elegida de brillo o volumen lleva la barra.
    // añadido en la 0.2.2
    fn alto_de(&self, i: usize) -> f32 {
        let con_barra =
            i == self.seleccion && matches!(self.resultados.get(i), Some(Resultado::Nivel { .. }));
        FILA + if con_barra { BARRA_EXTRA } else { 0.0 }
    }

    /// Dónde empieza la fila `i`, medido desde lo alto de la lista.
    // añadido en la 0.2.2
    fn y_fila(&self, i: usize) -> f32 {
        (0..i).map(|j| self.alto_de(j)).sum()
    }

    fn alto_lista(&self) -> f32 {
        self.y_fila(self.resultados.len().min(MAXIMO))
    }

    /// El alto que debe tener la tarjeta con lo que hay y lo elegido.
    // añadido en la 0.2.2
    fn alto_objetivo(&self) -> f32 {
        if self.confirmando.is_some() {
            FILA + MARGEN * 2.0
        } else if self.resultados.is_empty() {
            CAMPO
        } else {
            CAMPO + DIVISOR + SEPARACION + self.alto_lista() + MARGEN
        }
    }

    /// Cambia la elegida: la barra se abre en la nueva y se cierra en la
    /// anterior, y con ella la tarjeta.
    // añadido en la 0.2.2
    fn elegir(&mut self, i: usize) {
        self.seleccion = i;
        self.pendiente = None;
        self.señalada.señalar(None);
        self.elegida_y.ir_a(i as f32);
        self.alto.ir_a(self.alto_objetivo());
    }

    /// Qué fila cae en `y`, si cae en alguna.
    // 0.1.0
    // modificado en la 0.2.2 -- las filas ya no miden todas lo mismo
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho {
            return None;
        }
        let dentro = y - self.y_lista();
        if dentro < 0.0 {
            return None;
        }
        (0..self.resultados.len().min(MAXIMO)).find(|&i| {
            let desde = self.y_fila(i);
            (desde..desde + self.alto_de(i)).contains(&dentro)
        })
    }

    /// Dónde está la barra de la fila elegida, en la tarjeta: `(x, ancho)` del
    /// surco, entre los iconos de los extremos.
    // añadido en la 0.2.2
    fn surco(&self) -> (f32, f32) {
        let x0 = X_TEXTO + ICONO_BARRA + ENTRE_BARRA;
        (x0, self.ancho - DERECHA_FILA - (ICONO_BARRA + ENTRE_BARRA) - x0)
    }

    /// ¿Cae `(x, y)` en la barra de la fila elegida, con la holgura de un
    /// dedo? Solo hay barra en una fila de nivel elegida.
    // añadido en la 0.2.2
    fn en_barra(&self, x: f32, y: f32) -> bool {
        if !matches!(
            self.resultados.get(self.seleccion),
            Some(Resultado::Nivel { .. })
        ) {
            return false;
        }
        let (x0, ancho) = self.surco();
        let y0 = self.y_lista() + self.y_fila(self.seleccion) + FILA;
        (y0 - 4.0..=y0 + control::FILA_BARRA + 4.0).contains(&y)
            && (x0 - ENTRE_BARRA..=x0 + ancho + ENTRE_BARRA).contains(&x)
    }

    /// Lo que enseña la barra de la fila de nivel: lo pendiente si lo hay.
    // añadido en la 0.2.2
    fn nivel_visto(&self, que: Que, valor: u8) -> u8 {
        match self.pendiente {
            Some((q, v)) if q == que => v,
            _ => valor,
        }
    }

    /// Deja pendiente el nivel que corresponde a la `x` de la barra.
    // añadido en la 0.2.2
    fn poner_desde_x(&mut self, x: f32) -> bool {
        let Some(Resultado::Nivel { que, .. }) = self.resultados.get(self.seleccion) else {
            return false;
        };
        let que = *que;
        let (x0, ancho) = self.surco();
        let nivel = control::nivel_barra(x, x0, ancho).max(que.minimo());
        let cambia = self.pendiente != Some((que, nivel));
        self.pendiente = Some((que, nivel));
        cambia
    }

    /// ← y →: de 5 en 5 sobre lo pendiente o, si no hay, sobre lo que está.
    // añadido en la 0.2.2
    fn mover_nivel(&mut self, paso: i16) {
        let Some(Resultado::Nivel { que, valor, .. }) = self.resultados.get(self.seleccion) else {
            return;
        };
        let (que, base) = (*que, self.nivel_visto(*que, *valor));
        let nuevo = (i16::from(base) + paso).clamp(i16::from(que.minimo()), 100) as u8;
        self.pendiente = Some((que, nuevo));
    }

    fn iniciar_confirmacion(&mut self, titulo: String) {
        self.confirmando = Some((std::time::Instant::now(), titulo));
        self.pendiente = None;
        self.agarrada = false;
        self.todo_seleccionado = false;
        self.pendiente_archivos = None;
        self.en_curso = None;
        self.señalada = tema::Realce::nuevo();
        self.sello = tema::Transicion::nueva(0.4, tema::M_REBOTE);
        self.sello.ir_a(1.0);
        self.alto.ir_a(self.alto_objetivo());
    }

    /// Lo que Intro aplica en una fila de nivel: lo pendiente, y nada si no se
    /// ha tocado la barra.
    // añadido en la 0.2.2
    fn aplicar_nivel(&mut self) -> Option<Accion> {
        let (que, nivel) = self.pendiente.take()?;
        // La fila pasa a enseñar lo aplicado y arranca la confirmación.
        if let Some(Resultado::Nivel { valor, mudo, .. }) = self.resultados.get_mut(self.seleccion)
        {
            *valor = nivel;
            *mudo = false;
        }
        let titulo = match que {
            Que::Brillo => format!("{} {nivel} %", t("Brillo al", "Brightness to")),
            Que::Volumen if nivel == 0 => t("Volumen silenciado", "Volume muted").to_string(),
            Que::Volumen => format!("{} {nivel} %", t("Volumen al", "Volume to")),
        };
        self.iniciar_confirmacion(titulo);
        Some(que.accion(nivel))
    }

    /// ¿Se está enseñando la confirmación? Con ella el buscador sigue abierto
    /// tras la acción.
    // añadido en la 0.2.2
    pub fn confirmando(&self) -> bool {
        self.confirmando.is_some()
    }

    /// ¿Ya se ha visto lo bastante? El shell cierra entonces el buscador.
    // añadido en la 0.2.2
    pub fn confirmacion_vencida(&self) -> bool {
        self.confirmando
            .as_ref()
            .is_some_and(|(desde, _)| desde.elapsed() >= CONFIRMACION)
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if self.confirmando.is_some() {
            return false;
        }
        // Agarrada, la barra sigue a la mano aunque salga de la tarjeta.
        if self.agarrada {
            return punto.is_some_and(|(x, _)| self.poner_desde_x(x));
        }
        let nueva = punto.and_then(|(x, y)| self.fila_en(x, y));
        self.señalada.señalar(nueva)
    }

    // 0.1.0
    // modificado en la 0.2.0 -- aplica lo que la acción no lleva consigo
    // modificado en la 0.2.2 -- las menciones entran en su ámbito y la barra de nivel se agarra
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if self.confirmando.is_some() {
            return None;
        }
        let i = self.fila_en(x, y)?;
        if let Some(Resultado::Mencion { ambito, .. }) = self.resultados.get(i) {
            let ambito = *ambito;
            self.entrar(ambito, String::new());
            return None;
        }
        if matches!(self.resultados.get(i), Some(Resultado::Nivel { .. })) {
            if i != self.seleccion {
                self.elegir(i);
            } else if self.en_barra(x, y) {
                self.agarrada = true;
                self.poner_desde_x(x);
            } else {
                return self.aplicar_nivel();
            }
            return None;
        }
        let accion = self.resultados.get(i)?.accion().map(aplicar)?;
        if matches!(self.resultados.get(i), Some(Resultado::Dato { .. })) {
            self.elegir(i);
            self.iniciar_confirmacion(
                t("Copiado al portapapeles", "Copied to clipboard").to_string(),
            );
        }
        Some(accion)
    }

    /// Inserta texto limpio en la posición del cursor, sustituyendo toda la
    /// consulta si estaba seleccionada con `Ctrl+A`.
    fn insertar_cadena(&mut self, texto: &str) {
        let mut limpio = String::with_capacity(texto.len());
        for c in texto.chars() {
            if matches!(c, '\r' | '\n' | '\t') {
                limpio.push(' ');
            } else if !c.is_control() {
                limpio.push(c);
            }
        }
        if limpio.is_empty() {
            return;
        }
        if std::mem::take(&mut self.todo_seleccionado) {
            self.consulta.clear();
            self.cursor = 0;
        }
        let pos = self.cursor.min(self.consulta.len());
        self.consulta.insert_str(pos, &limpio);
        self.cursor = pos + limpio.len();
        self.buscar();
    }

    fn mover_cursor_izquierda(&mut self) {
        self.todo_seleccionado = false;
        self.cursor = self.consulta.floor_char_boundary(self.cursor);
        if self.cursor == 0 {
            return;
        }
        if let Some((i, _)) = self.consulta[..self.cursor].char_indices().next_back() {
            self.cursor = i;
        }
    }

    fn mover_cursor_derecha(&mut self) {
        self.todo_seleccionado = false;
        self.cursor = self.consulta.floor_char_boundary(self.cursor);
        if self.cursor >= self.consulta.len() {
            return;
        }
        if let Some(c) = self.consulta[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    /// Los controles del buscador para Orca / AT-SPI2.
    pub fn accesible(&self) -> Vec<FilaAccesible> {
        if let Some((_, titulo)) = &self.confirmando {
            return vec![FilaAccesible {
                id: "confirmacion".into(),
                rol: accesskit::Role::Status,
                nombre: titulo.clone(),
                valor: t("Aplicado", "Applied").into(),
                rect: (MARGEN, MARGEN, self.ancho - MARGEN * 2.0, FILA),
                marcada: None,
                seleccionada: false,
            }];
        }
        let mut nodos = Vec::with_capacity(1 + self.resultados.len().min(MAXIMO));
        let ayuda = match self.ambito {
            Ambito::Todo => t(
                "Buscar apps, ajustes o acciones",
                "Search apps, settings or actions",
            ),
            Ambito::Zenith => t("Pide algo a Zenith", "Ask Zenith anything"),
            Ambito::Archivos => t("Nombre del archivo", "File name"),
        };
        nodos.push(FilaAccesible {
            id: "campo".into(),
            rol: accesskit::Role::TextInput,
            nombre: ayuda.into(),
            valor: self.consulta.clone(),
            rect: (0.0, 0.0, self.ancho, CAMPO),
            marcada: None,
            seleccionada: false,
        });
        for (i, r) in self.resultados.iter().take(MAXIMO).enumerate() {
            nodos.push(FilaAccesible {
                id: format!("fila-{i}"),
                rol: accesskit::Role::ListBoxOption,
                nombre: r.nombre().to_string(),
                valor: r.detalle(),
                rect: (
                    MARGEN,
                    self.y_lista() + self.y_fila(i),
                    self.ancho - MARGEN * 2.0,
                    self.alto_de(i),
                ),
                marcada: None,
                seleccionada: i == self.seleccion,
            });
        }
        nodos
    }

    /// Se suelta la barra. `true` si hay que repintar.
    // añadido en la 0.2.2
    pub fn soltar(&mut self) -> bool {
        std::mem::take(&mut self.agarrada)
    }

    pub fn agarrado(&self) -> bool {
        self.agarrada
    }

    /// Para el reconocimiento táctil: ¿es aquí un arrastre de la barra?
    // añadido en la 0.2.2
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        self.confirmando.is_none() && self.en_barra(x, y)
    }

    // 0.1.0
    // modificado en la 0.2.0 -- Intro aplica lo que la acción no lleva consigo
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        if self.confirmando.is_some() {
            return Tecla::Consumida;
        }
        use crate::TeclaPulsada as T;
        let visibles = self.resultados.len().min(MAXIMO);
        if self.todo_seleccionado
            && matches!(
                tecla,
                T::Caracter(c) if !c.is_control()
            )
            || (self.todo_seleccionado
                && matches!(
                    tecla,
                    T::Retroceso | T::Suprimir | T::BorrarTodo | T::Pegar(_)
                ))
        {
            self.todo_seleccionado = false;
            self.consulta.clear();
            self.cursor = 0;
            match tecla {
                T::Caracter(c) if !c.is_control() => self.consulta.push(c),
                T::Pegar(s) => {
                    self.insertar_cadena(&s);
                    return Tecla::Consumida;
                }
                _ => {}
            }
            self.cursor = self.consulta.len();
            self.buscar();
            return Tecla::Consumida;
        }
        match tecla {
            T::SeleccionarTodo => {
                self.todo_seleccionado = !self.consulta.is_empty();
                self.cursor = self.consulta.len();
                Tecla::Consumida
            }
            // Esc borra lo escrito antes de cerrar, como el launchpad.
            T::Escape if !self.consulta.is_empty() => {
                self.todo_seleccionado = false;
                self.consulta.clear();
                self.cursor = 0;
                self.buscar();
                Tecla::Consumida
            }
            T::Escape | T::Retroceso if self.ambito != Ambito::Todo && self.consulta.is_empty() => {
                self.entrar(Ambito::Todo, String::new());
                Tecla::Consumida
            }
            T::Escape => Tecla::Cerrar,
            T::Pegar(texto) => {
                self.insertar_cadena(&texto);
                Tecla::Consumida
            }
            // «zenith» + espacio: la palabra se vuelve píldora.
            T::Caracter(' ')
                if self.ambito == Ambito::Todo && self.cursor == self.consulta.len() =>
            {
                match Ambito::por_palabra(self.consulta.trim()) {
                    Some(ambito) => self.entrar(ambito, String::new()),
                    None => self.insertar_cadena(" "),
                }
                Tecla::Consumida
            }
            T::Tabulador => {
                match self.resultados.get(self.seleccion) {
                    Some(Resultado::Ambito { ambito, texto, .. }) => {
                        let (ambito, texto) = (*ambito, texto.clone());
                        self.entrar(ambito, texto);
                    }
                    Some(Resultado::Mencion { ambito, .. }) => {
                        let ambito = *ambito;
                        self.entrar(ambito, String::new());
                    }
                    _ => {}
                }
                Tecla::Consumida
            }
            // Elegir una mención pone su píldora, como Tab.
            T::Intro
                if matches!(
                    self.resultados.get(self.seleccion),
                    Some(Resultado::Mencion { .. })
                ) =>
            {
                if let Some(Resultado::Mencion { ambito, .. }) = self.resultados.get(self.seleccion)
                {
                    let ambito = *ambito;
                    self.entrar(ambito, String::new());
                }
                Tecla::Consumida
            }
            // Sin tocar la barra no hay nada que aplicar.
            T::Intro
                if matches!(
                    self.resultados.get(self.seleccion),
                    Some(Resultado::Nivel { .. })
                ) =>
            {
                match self.aplicar_nivel() {
                    Some(accion) => Tecla::Hacer(accion),
                    None => Tecla::Consumida,
                }
            }
            T::Izquierda | T::Derecha
                if matches!(
                    self.resultados.get(self.seleccion),
                    Some(Resultado::Nivel { .. })
                ) =>
            {
                self.mover_nivel(if tecla == T::Izquierda { -5 } else { 5 });
                Tecla::Consumida
            }
            T::Izquierda => {
                self.mover_cursor_izquierda();
                Tecla::Consumida
            }
            T::Derecha => {
                self.mover_cursor_derecha();
                Tecla::Consumida
            }
            T::Inicio => {
                self.todo_seleccionado = false;
                self.cursor = 0;
                Tecla::Consumida
            }
            T::Fin => {
                self.todo_seleccionado = false;
                self.cursor = self.consulta.len();
                Tecla::Consumida
            }
            T::Intro
                if matches!(
                    self.resultados.get(self.seleccion),
                    Some(Resultado::Ambito {
                        ambito: Ambito::Archivos,
                        ..
                    })
                ) =>
            {
                let texto = std::mem::take(&mut self.consulta);
                self.cursor = 0;
                self.entrar(Ambito::Archivos, texto);
                Tecla::Consumida
            }
            T::Caracter(c) if !c.is_control() => {
                let mut buf = [0u8; 4];
                self.insertar_cadena(c.encode_utf8(&mut buf));
                Tecla::Consumida
            }
            T::Retroceso => {
                let pos = self.cursor.min(self.consulta.len());
                if pos > 0
                    && let Some((prev, _)) = self.consulta[..pos].char_indices().next_back()
                {
                    self.consulta.drain(prev..pos);
                    self.cursor = prev;
                    self.buscar();
                }
                Tecla::Consumida
            }
            T::Suprimir => {
                let pos = self.cursor.min(self.consulta.len());
                if pos < self.consulta.len()
                    && let Some(c) = self.consulta[pos..].chars().next()
                {
                    self.consulta.drain(pos..pos + c.len_utf8());
                    self.buscar();
                }
                Tecla::Consumida
            }
            // Ctrl+Retroceso borra la última palabra antes del cursor.
            T::BorrarTodo => {
                let pos = self.cursor.min(self.consulta.len());
                if pos > 0 {
                    let mut prefijo = self.consulta[..pos].to_string();
                    borrar_palabra(&mut prefijo);
                    let nuevo_cursor = prefijo.len();
                    prefijo.push_str(&self.consulta[pos..]);
                    self.consulta = prefijo;
                    self.cursor = nuevo_cursor;
                    self.buscar();
                }
                Tecla::Consumida
            }
            T::Abajo if visibles > 0 => {
                self.todo_seleccionado = false;
                self.elegir((self.seleccion + 1) % visibles);
                Tecla::Consumida
            }
            T::Arriba if visibles > 0 => {
                self.todo_seleccionado = false;
                self.elegir((self.seleccion + visibles - 1) % visibles);
                Tecla::Consumida
            }
            T::Intro => match self
                .resultados
                .get(self.seleccion)
                .and_then(|r| r.accion())
                .map(aplicar)
            {
                Some(accion) => {
                    if matches!(self.resultados.get(self.seleccion), Some(Resultado::Dato { .. })) {
                        self.iniciar_confirmacion(
                            t("Copiado al portapapeles", "Copied to clipboard").to_string(),
                        );
                    }
                    Tecla::Hacer(accion)
                }
                // Un dato sin valor copiable no cierra la ventana.
                None => Tecla::Consumida,
            },
            // El buscador es modal: lo que no entiende tampoco es para la
            // ventana de debajo, que no se ve y no sabe que se escribe aquí.
            _ => Tecla::Consumida,
        }
    }

    /// Lanza `plocate` cuando vence el debounce y recoge resultados al llegar.
    /// En Zenith van delante de «Preguntar a Zenith», que se queda.
    // añadido en la 0.2.2
    pub fn recoger(&mut self) {
        if self.confirmando.is_some() {
            return;
        }
        if let Some((_, desde)) = &self.pendiente_archivos
            && desde.elapsed() >= DEBOUNCE_ARCHIVOS
            && let Some((nombre, _)) = self.pendiente_archivos.take()
        {
            let (emisor, receptor) = std::sync::mpsc::channel();
            let buscado = nombre.clone();
            std::thread::spawn(move || {
                let _ = emisor.send(buscar_archivos(&buscado));
            });
            self.en_curso = Some((nombre, receptor));
        }
        let Some((_, receptor)) = &self.en_curso else {
            return;
        };
        let rutas = match receptor.try_recv() {
            Ok(rutas) => rutas,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        let Some((nombre, _)) = self.en_curso.take() else {
            return;
        };
        let mut vistas: std::collections::HashSet<std::path::PathBuf> = self
            .resultados
            .iter()
            .filter_map(|r| match r {
                Resultado::Archivo { ruta, .. } => Some(ruta.clone()),
                _ => None,
            })
            .collect();
        let rutas_nuevas: Vec<std::path::PathBuf> =
            rutas.into_iter().filter(|r| vistas.insert(r.clone())).collect();
        let hay_alguna = !rutas_nuevas.is_empty()
            || self
                .resultados
                .iter()
                .any(|r| matches!(r, Resultado::Archivo { .. }));
        let mut nuevos: Vec<Resultado> = Vec::new();
        if self.ambito == Ambito::Zenith
            && !self
                .resultados
                .iter()
                .any(|r| matches!(r, Resultado::Respuesta { .. }))
        {
            nuevos.push(match rutas_nuevas.first() {
                Some(ruta) => respuesta_zenith(ruta),
                None if !hay_alguna => Resultado::Dato {
                    titulo: format!("{} «{nombre}»", t("No encuentro", "Can't find")),
                    valor: t(
                        "Ni en tu carpeta personal · pregúntale a Zenith abajo",
                        "Not in your home folder · ask Zenith below",
                    )
                    .to_string(),
                    copiar: None,
                    icono: icono_zenith(),
                },
                None => {
                    return;
                }
            });
        }
        let existentes = self
            .resultados
            .iter()
            .filter(|r| matches!(r, Resultado::Archivo { .. } | Resultado::Respuesta { .. }))
            .count();
        let hueco = MAXIMO
            .saturating_sub(existentes)
            .saturating_sub(nuevos.len())
            .saturating_sub(usize::from(self.ambito == Ambito::Zenith));
        nuevos.extend(rutas_nuevas.into_iter().take(hueco).map(resultado_archivo));
        if nuevos.is_empty() {
            return;
        }
        self.resultados.splice(existentes..existentes, nuevos);
        self.resultados.truncate(MAXIMO);
        self.seleccion = 0;
        self.señalada = tema::Realce::nuevo();
        self.elegida_y.fijar(0.0);
        self.alto.ir_a(self.alto_objetivo());
    }

    fn ancho_disponible_campo(&self) -> f32 {
        let pildora = |nombre: &str| {
            crate::widget::ancho_con_fuente(
                nombre,
                16.0,
                super::control::peso(iced_core::font::Weight::Semibold),
            ) + 28.0
                + 12.0
        };
        let izquierda = match self.ambito {
            Ambito::Todo => 26.0 + 12.0,
            ambito => pildora(ambito.nombre()),
        };
        let derecha = self
            .resultados
            .get(self.seleccion)
            .and_then(Resultado::destino)
            .map_or(0.0, pildora);
        (self.ancho - 40.0 - izquierda - derecha - 6.0).max(80.0)
    }

    /// Partes antes y después del cursor que caben entre las píldoras,
    /// manteniendo siempre el cursor visible con `…` en los extremos recortados.
    fn partes_visibles(&self, cabe: f32) -> (String, String) {
        let mide = |t: &str| crate::widget::ancho_de(t, TEXTO_CAMPO);
        let pos = self.cursor.min(self.consulta.len());
        if mide(&self.consulta) <= cabe {
            return (self.consulta[..pos].to_string(), self.consulta[pos..].to_string());
        }
        let bordes: Vec<usize> = self
            .consulta
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(self.consulta.len()))
            .collect();
        let cur_idx = bordes
            .iter()
            .position(|&b| b >= pos)
            .unwrap_or(bordes.len() - 1);
        let mut izq_idx = 0usize;
        let mut der_idx = bordes.len() - 1;
        let armar = |ii: usize, di: usize| -> (String, String) {
            let b_izq = bordes[ii];
            let b_der = bordes[di];
            let pre = if ii > 0 {
                format!("…{}", &self.consulta[b_izq..pos])
            } else {
                self.consulta[b_izq..pos].to_string()
            };
            let suf = if di + 1 < bordes.len() {
                format!("{}…", &self.consulta[pos..b_der])
            } else {
                self.consulta[pos..b_der].to_string()
            };
            (pre, suf)
        };
        loop {
            let (pre, suf) = armar(izq_idx, der_idx);
            if mide(&pre) + mide(&suf) <= cabe {
                return (pre, suf);
            }
            let dist_izq = cur_idx.saturating_sub(izq_idx);
            let dist_der = der_idx.saturating_sub(cur_idx);
            if dist_izq == 0 && dist_der == 0 {
                return (pre, suf);
            }
            if dist_der > dist_izq && der_idx > cur_idx {
                der_idx -= 1;
            } else if izq_idx < cur_idx {
                izq_idx += 1;
            } else if der_idx > cur_idx {
                der_idx -= 1;
            }
        }
    }

    // añadido en la 0.2.2
    fn entrar(&mut self, ambito: Ambito, texto: String) {
        self.ambito = ambito;
        self.todo_seleccionado = false;
        self.cursor = texto.len();
        self.consulta = texto;
        self.buscar();
    }

    /// Tiempo restante para cerrar automáticamente la confirmación actual, si
    /// la hay, para despertar una sola vez sin repintar fotogramas en reposo.
    pub fn proxima_alarma(&self) -> Option<std::time::Duration> {
        self.confirmando
            .as_ref()
            .map(|(desde, _)| CONFIRMACION.saturating_sub(desde.elapsed()))
    }

    // modified 0.2.0 -- elegida, realce y alto
    pub fn animando(&self) -> bool {
        self.elegida_y.animando()
            || self.señalada.animando()
            || self.alto.animando()
            || self.pendiente_archivos.is_some()
            || self.en_curso.is_some()
            || (self.confirmando.is_some() && self.sello.animando())
    }

    // 0.1.0
    // modified 0.1.1 -- campo más grande y texto de ayuda nuevo
    pub fn view(&self) -> PanelElement<'_> {
        let colores = Colores::actuales();
        if let Some((_, titulo)) = &self.confirmando {
            return container(self.confirmacion(titulo, colores))
                .width(Length::Fixed(self.ancho))
                .height(Length::Fixed(self.alto.valor()))
                .center_x(Length::Fixed(self.ancho))
                .center_y(Length::Fixed(self.alto.valor()))
                .clip(true)
                .style(move |_theme| container::Style {
                    background: Some(colores.tarjeta.into()),
                    border: Border {
                        radius: tema::R_DIALOGO.into(),
                        width: 1.0,
                        color: colores.borde,
                    },
                    ..Default::default()
                })
                .into();
        }
        let lupa = icono::propio("buscar")
            .map(|ic| icono::ver_teñido_propio(&ic, 26.0, colores.secundario))
            .unwrap_or_else(crate::widget::vacio);
        let cabe = self.ancho_disponible_campo();
        let escrito: PanelElement<'_> = if self.consulta.is_empty() {
            text(match self.ambito {
                Ambito::Todo => t(
                    "Buscar apps, ajustes o acciones",
                    "Search apps, settings or actions",
                ),
                Ambito::Zenith => t("Pide algo a Zenith", "Ask Zenith anything"),
                Ambito::Archivos => t("Nombre del archivo", "File name"),
            })
            .size(TEXTO_CAMPO)
            .color(colores.secundario)
            .into()
        } else if self.todo_seleccionado {
            let acento = colores.acento;
            let visible = recortar_ancho(&self.consulta, cabe, TEXTO_CAMPO);
            container(
                text(visible)
                    .size(TEXTO_CAMPO)
                    .color(tema::tinta_sobre(acento)),
            )
            .padding([0, 2])
            .style(move |_theme| container::Style {
                background: Some(acento.into()),
                border: Border {
                    radius: 3.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
        } else {
            let (antes, despues) = self.partes_visibles(cabe);
            let acento = colores.acento;
            // Barra de cursor propia: evita depender de glifos Unicode de caja
            // («▏») que en algunas fuentes se ven desalineados o como tofu.
            let barra_cursor = container(Space::new())
                .width(Length::Fixed(2.0))
                .height(Length::Fixed(24.0))
                .style(move |_theme| container::Style {
                    background: Some(acento.into()),
                    border: Border {
                        radius: 1.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                });
            let mut fila_texto = row![].align_y(Vertical::Center);
            if !antes.is_empty() {
                fila_texto = fila_texto.push(text(antes).size(TEXTO_CAMPO).color(colores.texto));
            }
            fila_texto = fila_texto.push(barra_cursor);
            if !despues.is_empty() {
                fila_texto = fila_texto.push(text(despues).size(TEXTO_CAMPO).color(colores.texto));
            }
            fila_texto.into()
        };
        // modificado en la 0.2.2 -- píldora del ámbito a la izquierda y de la app destino a la derecha
        let izquierda: PanelElement<'_> = match self.ambito {
            Ambito::Todo => lupa,
            ambito => pildora(ambito.nombre(), colores),
        };
        let destino: PanelElement<'_> = match self
            .resultados
            .get(self.seleccion)
            .and_then(Resultado::destino)
        {
            Some(app) => pildora(app, colores),
            None => crate::widget::vacio(),
        };
        let campo = container(
            row![
                izquierda,
                Space::new().width(Length::Fixed(12.0)),
                escrito,
                Space::new().width(Length::Fill),
                destino,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho))
        .height(Length::Fixed(CAMPO))
        // El texto empieza donde empieza el contenido de las filas. Centrar el
        // conjunto hacía que la lupa y el texto saltasen horizontalmente al
        // escribir, porque su anchura cambia con cada carácter.
        .padding([0, 20])
        .align_y(Vertical::Center);

        let mut contenido = column![campo];
        if !self.resultados.is_empty() {
            // El divisor separa el campo de la lista con la misma línea que usa
            // el pie de las tarjetas de conectividad.
            contenido = contenido.push(
                container(Space::new().height(Length::Fixed(DIVISOR)))
                    .width(Length::Fixed(self.ancho))
                    .style(move |_theme| container::Style {
                        background: Some(colores.divisor.into()),
                        ..Default::default()
                    }),
            );
            contenido = contenido.push(Space::new().height(Length::Fixed(SEPARACION)));
            let mut lista = column![];
            for (i, resultado) in self.resultados.iter().take(MAXIMO).enumerate() {
                lista = lista.push(self.fila(i, resultado, colores));
            }
            // El acento se reserva para lo que Intro va a lanzar; va debajo de
            // las filas para que el realce del ratón no lo tape.
            let acento = tema::alfa(colores.acento, 0.10);
            let (placa_y, placa_alto) = self.placa();
            let elegida = column![
                Space::new().height(Length::Fixed(placa_y)),
                container(Space::new())
                    .width(Length::Fixed(self.ancho - MARGEN * 2.0))
                    .height(Length::Fixed(placa_alto))
                    .style(move |_theme| container::Style {
                        background: Some(acento.into()),
                        border: Border {
                            radius: tema::R_ITEM_POPOVER.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            ]
            .padding([0, MARGEN as u16])
            // `stack!` mide lo que su primera capa: sin el alto de la lista
            // entera, recortaba las filas por debajo de la elegida.
            .height(Length::Fixed(self.alto_lista()));
            contenido = contenido.push(stack![elegida, lista]);
            // El respiro bajo la última fila: sin él su realce llega al borde y
            // se come la esquina redondeada de la tarjeta. Va aquí y no como
            // alto de más en `size()`, que es donde estaba: allí eran píxeles
            // transparentes fuera de la tarjeta y descuadraban el centrado.
            contenido = contenido.push(Space::new().height(Length::Fixed(MARGEN)));
        }

        container(contenido)
            .width(Length::Fixed(self.ancho))
            .height(Length::Fixed(self.alto.valor()))
            .clip(true)
            .style(move |_theme| container::Style {
                // El HIG reserva el cristal para la superficie temporal que
                // está detrás (el overlay). La tarjeta propia permanece
                // legible y opaca en ambos temas.
                background: Some(colores.tarjeta.into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: colores.borde,
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.0
    // modified 0.1.1 -- texto de las filas un punto más grande
    fn fila<'a>(
        &'a self,
        i: usize,
        resultado: &'a Resultado,
        colores: Colores,
    ) -> PanelElement<'a> {
        // El ratón por encima solo levanta un velo de tinta, y no sobre la
        // elegida: si las dos cosas se pintaran igual, pasar el ratón parecería
        // cambiar lo que va a ejecutarse.
        let fondo = if i == self.seleccion {
            iced_core::Color::TRANSPARENT
        } else {
            Color {
                a: colores.hover.a * self.señalada.intensidad(i),
                ..colores.hover
            }
        };
        let dibujo: PanelElement<'a> = match resultado.icono() {
            // Los de aplicación no se tiñen —el zorro de Firefox en silueta
            // blanca no lo quiere nadie—; los propios sí, que son monocromos.
            // Las acciones y los datos ya llegan como insignia (ver [`insignia`]),
            // con su color: van sin teñir, igual que los de aplicación.
            Some(ic) => icono::ver(ic, ICONO, ICONO),
            // `ver` ata el elemento al préstamo; aquí la insignia es temporal y
            // basta con clonar el `Handle`, que es lo que hace `ver` por dentro.
            None => match insignia("terminal") {
                Some(Icono::Svg(handle)) => iced_widget::svg(handle)
                    .width(Length::Fixed(ICONO))
                    .height(Length::Fixed(ICONO))
                    .into(),
                _ => crate::widget::vacio(),
            },
        };
        // Lo escrito se marca en negrita en el principio del nombre, como en el
        // diseño: se ve por qué ha salido esa fila.
        let tiene_badge = i == self.seleccion && resultado.tiene_intro();
        let ancho_texto = (self.ancho
            - MARGEN * 2.0
            - (18.0 - MARGEN)
            - ICONO
            - 14.0
            - 14.0
            - if tiene_badge { 36.0 } else { 0.0 })
        .max(120.0);
        let nombre = recortar_ancho(resultado.nombre(), ancho_texto, 15.0);
        let escrito = self.consulta.trim();
        let marcadas = if self.ambito == Ambito::Todo
            && !escrito.is_empty()
            && !escrito.starts_with('@')
        {
            prefijo_coincidente(&nombre, escrito)
        } else {
            0
        };
        let titulo: PanelElement<'a> = if marcadas == 0 {
            text(nombre).size(15.0).color(colores.texto).into()
        } else {
            row![
                text(nombre[..marcadas].to_string())
                    .size(15.0)
                    .font(super::control::peso(iced_core::font::Weight::Semibold))
                    .color(colores.texto),
                text(nombre[marcadas..].to_string())
                    .size(15.0)
                    .color(colores.texto),
            ]
            .align_y(Vertical::Center)
            .into()
        };
        let textos = column![
            titulo,
            text(recortar_ancho(&resultado.detalle(), ancho_texto, 12.0))
                .size(12.0)
                .color(colores.secundario),
        ];
        // La tecla que hará la fila elegida.
        let intro: PanelElement<'a> = if tiene_badge {
            let (tarjeta, borde, texto) = (colores.tarjeta, colores.borde, colores.texto);
            container(
                text("↵")
                    .size(11.0)
                    .font(super::control::peso(iced_core::font::Weight::Semibold))
                    .color(texto),
            )
            .center_x(Length::Fixed(24.0))
            .center_y(Length::Fixed(22.0))
            .style(move |_theme| container::Style {
                background: Some(tarjeta.into()),
                border: Border {
                    radius: 6.0.into(),
                    width: 1.0,
                    color: borde,
                },
                ..Default::default()
            })
            .into()
        } else {
            crate::widget::vacio()
        };
        // El realce va **dentro** de un margen y con esquinas: pegado a los
        // bordes de la tarjeta, la última fila se comía su esquina redondeada.
        let linea = container(
            row![
                Space::new().width(Length::Fixed(18.0 - MARGEN)),
                dibujo,
                Space::new().width(Length::Fixed(14.0)),
                textos,
                Space::new().width(Length::Fill),
                intro,
                Space::new().width(Length::Fixed(14.0)),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho - MARGEN * 2.0))
        .height(Length::Fixed(FILA))
        .align_y(Vertical::Center);
        let linea: PanelElement<'a> = match &self.confirmando {
            Some((_, titulo)) if i == self.seleccion => {
                iced_widget::stack![linea, self.confirmacion(titulo, colores)].into()
            }
            _ => linea.into(),
        };
        let mut cuerpo = column![linea];
        if let (true, Resultado::Nivel { que, valor, glifo, .. }) = (i == self.seleccion, resultado) {
            cuerpo = cuerpo.push(self.barra(*que, self.nivel_visto(*que, *valor), glifo.as_ref()));
        }
        let fila = container(cuerpo)
            .width(Length::Fixed(self.ancho - MARGEN * 2.0))
            .height(Length::Fixed(self.alto_de(i)))
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_ITEM_POPOVER.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(fila)
            .width(Length::Fixed(self.ancho))
            .padding([0, MARGEN as u16])
            .into()
    }

    /// La línea de confirmación que tapa la de la fila: sello verde con el ✓ y
    /// lo que quedó. Opaca y del color de la placa de la elegida.
    // añadido en la 0.2.2
    fn confirmacion<'a>(&self, titulo: &'a str, colores: Colores) -> PanelElement<'a> {
        let fondo = tema::mezclar(colores.tarjeta, colores.acento, 0.10);
        let verde = if tema::es_claro() {
            Color::from_rgb8(0x24, 0x8a, 0x3d)
        } else {
            Color::from_rgb8(0x30, 0xd1, 0x58)
        };
        let escala_sello = self.sello.valor().clamp(0.0, 1.2);
        let lado = ICONO * escala_sello;
        let visto_lado = (18.0 * escala_sello).max(1.0);
        let visto = icono::desde_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>"##,
        );
        let visto: PanelElement<'a> = match visto {
            Icono::Svg(h) => iced_widget::svg(h)
                .width(Length::Fixed(visto_lado))
                .height(Length::Fixed(visto_lado))
                .into(),
            _ => Space::new().into(),
        };
        let sello = container(visto)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .center_x(Length::Fixed(lado))
            .center_y(Length::Fixed(lado))
            .style(move |_theme| container::Style {
                background: Some(verde.into()),
                border: Border {
                    radius: (lado / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(
            row![
                Space::new().width(Length::Fixed(18.0 - MARGEN)),
                // Hueco fijo del tamaño del icono: crecer no mueve el texto.
                container(sello)
                    .width(Length::Fixed(ICONO))
                    .height(Length::Fixed(ICONO))
                    .center_x(Length::Fixed(ICONO))
                    .center_y(Length::Fixed(ICONO)),
                Space::new().width(Length::Fixed(14.0)),
                column![
                    text(titulo)
                        .size(15.0)
                        .font(super::control::peso(iced_core::font::Weight::Semibold))
                        .color(colores.texto),
                    text(t("Aplicado", "Applied")).size(12.0).color(colores.secundario),
                ],
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho - MARGEN * 2.0))
        .height(Length::Fixed(FILA))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_ITEM_POPOVER.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Dónde está la placa de la elegida y cuánto mide, entre dos filas
    /// mientras se desliza: la altura también se interpola, porque una fila de
    /// nivel elegida mide más que las demás.
    // añadido en la 0.2.2
    fn placa(&self) -> (f32, f32) {
        let n = self.resultados.len().min(MAXIMO);
        if n == 0 {
            return (0.0, FILA);
        }
        let f = self.elegida_y.valor().clamp(0.0, (n - 1) as f32);
        let i = f.floor() as usize;
        let frac = f - i as f32;
        let (h0, h1) = (self.alto_de(i), self.alto_de((i + 1).min(n - 1)));
        (self.y_fila(i) + frac * h0, h0 + frac * (h1 - h0))
    }

    /// La barra de brillo o volumen, la de las tarjetas, bajo el texto de la
    /// fila: icono, surco y icono, con el mismo grueso que engorda al agarrarla.
    // añadido en la 0.2.2
    fn barra<'a>(&'a self, que: Que, nivel: u8, glifo: Option<&'a Icono>) -> PanelElement<'a> {
        let (_, ancho) = self.surco();
        let extremo = |lado: f32| -> PanelElement<'a> {
            let dibujo: PanelElement<'a> = match glifo {
                Some(ic) => icono::ver_teñido(ic, lado, Some(tema::texto())),
                None => Space::new().into(),
            };
            container(dibujo)
                .width(Length::Fixed(ICONO_BARRA))
                .height(Length::Fixed(control::FILA_BARRA))
                .center_x(Length::Fixed(ICONO_BARRA))
                .center_y(Length::Fixed(control::FILA_BARRA))
                .into()
        };
        let grueso = if self.agarrada {
            control::PISTA_AGARRADA
        } else {
            control::PISTA
        };
        let (menor, mayor) = match que {
            Que::Brillo => (15.0, 20.0),
            Que::Volumen => (17.0, 20.0),
        };
        column![
            row![
                Space::new().width(Length::Fixed(X_TEXTO - MARGEN)),
                extremo(menor),
                Space::new().width(Length::Fixed(ENTRE_BARRA)),
                control::barra(
                    ancho,
                    grueso,
                    f32::from(nivel) / 100.0,
                    tema::texto(),
                    tema::elevado(),
                    &[],
                    None,
                ),
                Space::new().width(Length::Fixed(ENTRE_BARRA)),
                extremo(mayor),
            ]
            .align_y(Vertical::Center),
            Space::new().height(Length::Fixed(10.0)),
        ]
        .into()
    }
}

/// La píldora de un ámbito o de una app: un relleno opaco mezclado sobre la
/// tarjeta (nada de translúcidos) y el nombre en seminegrita, como un token
/// de Spotlight.
// añadido en la 0.2.2
fn pildora(nombre: &str, colores: Colores) -> PanelElement<'_> {
    let fondo = tema::mezclar(colores.tarjeta, colores.acento, 0.16);
    container(
        text(nombre)
            .size(16.0)
            .font(super::control::peso(iced_core::font::Weight::Semibold))
            .color(tema::mezclar(colores.texto, colores.acento, 0.35)),
    )
    .padding([7, 14])
    .style(move |_theme| container::Style {
        background: Some(fondo.into()),
        border: Border {
            radius: tema::R_ITEM_POPOVER.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// A quién se le puede hablar con «@»: los que empiezan por lo escrito tras la
/// arroba. Con solo «@» salen todos.
// añadido en la 0.2.2
fn menciones(prefijo: &str) -> Vec<Resultado> {
    let p = apps::normalizar(prefijo);
    let todas: [(Ambito, &[&str], &'static str, Option<Icono>); 2] = [
        (
            Ambito::Zenith,
            &["zenith"],
            t(
                "Pídele algo: busca un archivo, abre una app…",
                "Ask for something: find a file, open an app…",
            ),
            icono_zenith(),
        ),
        (
            Ambito::Archivos,
            &["archivos", "archivo", "explorador", "files"],
            t(
                "Busca por nombre en tu carpeta personal",
                "Search by name in your home folder",
            ),
            icono::cargar("system-file-manager").or_else(|| icono::cargar("folder")),
        ),
    ];
    todas
        .into_iter()
        .filter(|(_, palabras, ..)| palabras.iter().any(|w| w.starts_with(&p)))
        .map(|(ambito, _, detalle, icono)| Resultado::Mencion {
            ambito,
            detalle,
            icono,
        })
        .collect()
}

/// Las filas que llevan lo escrito a Zenith o a archivos, al final de la
/// lista del ámbito general. Zenith solo con dos palabras o más: una sola
/// es casi siempre el nombre de una app y la fila estorbaría.
// añadido en la 0.2.2
fn puertas(consulta: &str, ocupadas: usize) -> Vec<Resultado> {
    let texto = consulta.trim();
    if texto.chars().count() < 3 || texto.starts_with('>') {
        return Vec::new();
    }
    let mut lista = Vec::new();
    if texto.split_whitespace().nth(1).is_some() {
        lista.push(Resultado::Ambito {
            ambito: Ambito::Zenith,
            texto: texto.to_string(),
            icono: icono_zenith(),
        });
    }
    lista.push(Resultado::Ambito {
        ambito: Ambito::Archivos,
        texto: texto.to_string(),
        icono: icono::cargar("system-file-manager").or_else(|| icono::cargar("folder")),
    });
    // Solo se ven seis filas: si no caben enteras no se meten a medias.
    lista.truncate(MAXIMO.saturating_sub(ocupadas));
    lista
}

/// Con la píldora de Zenith, la fila de preguntarle. Si lo escrito pide un
/// archivo por su nombre (`archivo_pedido`), los archivos llegan después y se
/// ponen delante: es lo que Zenith haría y aquí se ve al momento.
// añadido en la 0.2.2
fn en_zenith(consulta: &str) -> Vec<Resultado> {
    let texto = consulta.trim();
    if texto.is_empty() {
        return Vec::new();
    }
    // Ya dentro de Zenith no hay «Tab para seguir aquí»: es una acción más.
    vec![Resultado::Accion {
        nombre: t("Preguntar a Zenith", "Ask Zenith").to_string(),
        detalle: format!("«{texto}»"),
        icono: icono_zenith(),
        accion: preguntar_a_zenith(texto),
    }]
}

fn respuesta_zenith(ruta: &std::path::Path) -> Resultado {
    Resultado::Respuesta {
        titulo: format!(
            "{} {} {}",
            ruta.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default(),
            t("está en", "is in"),
            ruta.parent().map(con_tilde).unwrap_or_default()
        ),
        ruta: ruta.to_path_buf(),
        icono: icono_zenith(),
    }
}

/// Busca coincidencias en los archivos recientes de `recently-used.xbel` (sin
/// latencia de disco ni esperar a `updatedb`).
fn buscar_recientes(nombre: &str) -> Vec<std::path::PathBuf> {
    let q = apps::normalizar(nombre);
    let mut rutas: Vec<std::path::PathBuf> = apps::recientes_todos(40)
        .into_iter()
        .filter(|r| {
            r.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| apps::normalizar(n).contains(&q))
        })
        .collect();
    rutas.sort_by_cached_key(|ruta| {
        let empieza = ruta
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| apps::normalizar(n).starts_with(&q));
        (!empieza, ruta.components().count())
    });
    rutas.truncate(MAXIMO);
    rutas
}

/// Recorta texto midiendo su ancho real con `iced` en lugar de estimar por carácter.
fn recortar_ancho(texto: &str, max_ancho: f32, tamaño: f32) -> String {
    if crate::widget::ancho_de(texto, tamaño) <= max_ancho {
        return texto.to_string();
    }
    let cortes: Vec<usize> = texto
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(texto.len()))
        .collect();
    let (mut lo, mut hi) = (0usize, cortes.len() - 1);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidato = format!("{}…", &texto[..cortes[mid]]);
        if crate::widget::ancho_de(&candidato, tamaño) <= max_ancho {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        "…".into()
    } else {
        format!("{}…", &texto[..cortes[lo]])
    }
}

/// Índice en bytes de `nombre` donde termina el prefijo que casa con `escrito`.
fn prefijo_coincidente(nombre: &str, escrito: &str) -> usize {
    let q = apps::normalizar(escrito);
    let n_chars = q.chars().count();
    if n_chars == 0 {
        return 0;
    }
    let Some((ultimo_i, ultimo_c)) = nombre.char_indices().nth(n_chars - 1) else {
        return 0;
    };
    let fin = ultimo_i + ultimo_c.len_utf8();
    if apps::normalizar(&nombre[..fin]) == q {
        fin
    } else {
        0
    }
}

/// El nombre en «busca un archivo llamado X», «abre la carpeta X», «encuentra
/// X». Sin verbo de buscar o abrir no es una petición de archivo.
// añadido en la 0.2.2
fn archivo_pedido(texto: &str) -> Option<&str> {
    let bajo = texto.to_lowercase();
    if let Some(i) = bajo.find("llamad") {
        let resto = bajo[i..].split_once(' ')?.1.trim();
        if resto.is_empty() {
            return None;
        }
        // Si las longitudes coinciden preservamos las mayúsculas originales.
        if bajo.len() == texto.len() {
            let offset = texto.len() - resto.len();
            return Some(texto[offset..].trim());
        }
        return Some(texto.rsplit(' ').next()?.trim());
    }
    let verbos = [
        "busca ",
        "buscar ",
        "encuentra ",
        "abre ",
        "abrir ",
        "find ",
        "open ",
    ];
    let verbo = verbos.iter().find(|v| bajo.starts_with(*v))?;
    let mut nombre = if bajo.len() == texto.len() {
        texto[verbo.len()..].trim()
    } else {
        texto.split_once(' ')?.1.trim()
    };
    for articulo in [
        "el archivo ",
        "un archivo ",
        "la carpeta ",
        "una carpeta ",
        "the file ",
        "a file ",
        "de ",
        "of ",
    ] {
        if nombre.to_lowercase().starts_with(articulo) {
            nombre = nombre.get(articulo.len()..).unwrap_or(nombre).trim();
        }
    }
    (!nombre.is_empty()).then_some(nombre)
}

/// Archivos y carpetas de la carpeta personal cuyo nombre contiene lo
/// escrito, del índice de `plocate`. La expresión deja fuera lo que cuelga de
/// carpetas ocultas —la caché de una app no se busca a mano— y solo mira el
/// último componente: «hig» no saca todo lo de dentro de una carpeta «HIG».
/// Van primero los nombres que empiezan por lo escrito y, entre ellos, los
/// menos hondos: `~/Descargas/BookOS/BookOS-HIG` antes que un recurso del SDK
/// de Android que se llama «highlight».
// añadido en la 0.2.2
fn buscar_archivos(nombre: &str) -> Vec<std::path::PathBuf> {
    let Some(casa) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let casa = casa.to_string_lossy();
    let expresion = format!(
        "^{}/([^./][^/]*/)*([^./][^/]*)?{}[^/]*$",
        escapar_regex(&casa),
        escapar_regex(nombre)
    );
    let salida = match std::process::Command::new("plocate")
        .args(["-i", "-l", "200", "--regex", &expresion])
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(salida) => salida,
        Err(err) => {
            tracing::warn!("plocate: {err}");
            return Vec::new();
        }
    };
    let bajo = nombre.to_lowercase();
    let mut rutas: Vec<std::path::PathBuf> = String::from_utf8_lossy(&salida.stdout)
        .lines()
        .map(std::path::PathBuf::from)
        .collect();
    rutas.sort_by_cached_key(|ruta| {
        let empieza = ruta
            .file_name()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(&bajo));
        (!empieza, ruta.components().count())
    });
    rutas.truncate(MAXIMO);
    rutas
}

/// Lo escrito, literal dentro de una expresión POSIX extendida.
// añadido en la 0.2.2
fn escapar_regex(texto: &str) -> String {
    texto
        .chars()
        .flat_map(|c| {
            let especial = "\\.^$|?*+()[]{}".contains(c);
            especial
                .then_some('\\')
                .into_iter()
                .chain(std::iter::once(c))
        })
        .collect()
}

// añadido en la 0.2.2
fn resultado_archivo(ruta: std::path::PathBuf) -> Resultado {
    let icono = if ruta.is_dir() {
        icono::cargar("folder")
    } else {
        icono::cargar("text-x-generic")
    };
    Resultado::Archivo { ruta, icono }
}

/// Calcula el ancho del buscador en coordenadas lógicas.
///
/// La pantalla física no entra aquí: `ShellHost` ya la ha dividido por la
/// escala del monitor. Por eso un 4K a 200 % y un 4K a 100 % se comportan como
/// dos escritorios lógicos distintos, que es exactamente lo que espera el
/// usuario. El límite superior conserva la lectura de una fila y evita que el
/// buscador se convierta en una barra gigantesca en un ultrawide.
// 0.1.0
fn ancho_para(pantalla_logica: f32) -> f32 {
    (pantalla_logica * ANCHO / 1920.0).clamp(ANCHO_MIN, ANCHO_MAX)
}

/// La lista de resultados para una consulta. Suelta para poder probarla sin
/// leer los `.desktop` de la máquina.
// 0.1.0
// modified 0.1.1 -- sugerencias por uso con el campo vacío, calculadora y acciones mezcladas con las apps
// modificado en la 0.2.0 -- «brillo 30» y «volumen 50» arriba del todo
fn resultados(
    apps: &[apps::App],
    usos: &std::collections::HashMap<String, u32>,
    abiertas: &[(String, String)],
    consulta: &str,
) -> Vec<Resultado> {
    let app_id = |exec: &str| {
        abiertas
            .iter()
            .find(|(e, _)| e.trim() == exec.trim())
            .map(|(_, id)| id.clone())
    };
    let consulta = consulta.trim();
    // Con el campo vacío, lo que más abres: es lo que se busca casi siempre, y
    // así Meta+Espacio e Intro relanza tu aplicación de cada día.
    if consulta.is_empty() {
        let mut usadas: Vec<(&apps::App, u32)> = apps
            .iter()
            .filter_map(|app| Some((app, *usos.get(&app.exec)?)))
            .collect();
        usadas.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.nombre.cmp(&b.0.nombre)));
        return usadas
            .into_iter()
            .take(SUGERENCIAS)
            .map(|(app, _)| resultado_app(app, app_id(&app.exec)))
            .collect();
    }
    // Con `>` delante no se busca nada: es una orden, tal cual, con sus
    // argumentos. Es lo que permite escribir algo que además es el nombre de
    // una aplicación y que se ejecute el binario.
    if let Some(orden) = consulta.strip_prefix('>') {
        let orden = orden.trim();
        return if orden.is_empty() {
            Vec::new()
        } else {
            vec![Resultado::Comando(orden.to_string())]
        };
    }

    let mut lista = Vec::new();
    if let Some(ajuste) = ajuste(consulta) {
        lista.push(ajuste);
    }
    if let Some(valor) = calcular(consulta) {
        let numero = formatear_numero(valor);
        lista.push(Resultado::Dato {
            titulo: format!("= {numero}"),
            valor: format!(
                "{} · {}",
                t("Resultado del cálculo", "Calculation result"),
                t("Intro copia el valor", "Enter copies the value")
            ),
            copiar: Some(numero),
            icono: insignia("control"),
        });
    }
    if let Some(dato) = estado(consulta) {
        lista.push(dato);
    }

    // Aplicaciones y acciones compiten en la misma lista con la regla del
    // launchpad: primero lo que encaja bien (prefijo, palabra, iniciales), y
    // dentro de eso lo que ya está abierto y luego lo más usado. Antes salía
    // una sola acción y siempre delante: «son» daba Sonido, pero «so» ya no
    // daba nada.
    let mut encajan: Vec<(bool, bool, std::cmp::Reverse<u32>, u32, Resultado)> = apps
        .iter()
        .filter_map(|app| {
            let p = apps::puntuar(app, consulta)?;
            let n = usos.get(&app.exec).copied().unwrap_or(0);
            let abierta = app_id(&app.exec);
            Some((
                p > 3,
                abierta.is_none(),
                std::cmp::Reverse(n),
                p,
                resultado_app(app, abierta),
            ))
        })
        .collect();
    encajan.extend(atajos().into_iter().filter_map(|atajo| {
        let p = puntuar_atajo(&atajo, consulta)?;
        let n = match &atajo.accion {
            Accion::Lanzar(exec) => usos.get(exec).copied().unwrap_or(0),
            _ => 0,
        };
        Some((
            p > 3,
            true,
            std::cmp::Reverse(n),
            p,
            Resultado::Accion {
                nombre: atajo.nombre,
                detalle: atajo.detalle.to_string(),
                icono: match atajo.accion {
                    Accion::Apariencia { .. } => {
                        crate::icono::insignia_con(atajo.icono, COLOR_TEMAS)
                    }
                    _ => insignia(atajo.icono),
                },
                accion: atajo.accion,
            },
        ))
    }));
    encajan.sort_by(|a, b| {
        (a.0, a.1, a.2, a.3)
            .cmp(&(b.0, b.1, b.2, b.3))
            .then_with(|| a.4.nombre().cmp(b.4.nombre()))
    });
    lista.extend(encajan.into_iter().take(MAXIMO).map(|(.., r)| r));

    // Y si además es un ejecutable, se ofrece: `htop` no tiene `.desktop` y sin
    // esto habría que acordarse del prefijo `>` para lanzarlo.
    //
    // Va **el primero** cuando lo escrito es exactamente el nombre del binario:
    // quien escribe «sh» entero quiere `sh`, no una aplicación que lo lleve
    // dentro del nombre. Escribir un prefijo suyo lo deja al final, donde
    // estorba menos que una lista de aplicaciones desplazada.
    let primera_palabra = consulta.split_whitespace().next().unwrap_or(consulta);
    if en_el_path(primera_palabra) {
        let comando = Resultado::Comando(consulta.to_string());
        let consulta_norm = apps::normalizar(consulta);
        let hay_app_preferente = lista.iter().any(|r| {
            matches!(r, Resultado::App { nombre, .. } if apps::normalizar(nombre).starts_with(&consulta_norm))
        });
        if primera_palabra == consulta && !hay_app_preferente {
            lista.insert(0, comando);
        } else {
            lista.push(comando);
        }
    }
    lista
}

// 0.1.1
fn resultado_app(app: &apps::App, abierta: Option<String>) -> Resultado {
    Resultado::App {
        nombre: app.nombre.clone(),
        detalle: app.detalle.clone(),
        exec: app.exec.clone(),
        icono: icono::cargar(&app.icono),
        abierta,
    }
}

/// Una acción del escritorio que se puede pedir por su nombre.
struct Atajo {
    nombre: String,
    detalle: &'static str,
    icono: &'static str,
    /// Otras formas de llamarlo: quien quiere quitar el adorno no tiene por
    /// qué saber cómo lo llamamos aquí.
    claves: &'static [&'static str],
    accion: Accion,
}

/// Las acciones del escritorio. No se ejecutan como texto arbitrario: cada una
/// viaja como [`Accion`] hasta el compositor, lo que permite que apagar,
/// bloquear o abrir una tarjeta mantenga el mismo control de foco y entorno
/// que el resto del shell.
///
/// Se construye en cada pulsación porque unas cuantas dependen del estado
/// —los efectos, la luz nocturna, el tema—; son unas cuarenta y cuesta menos
/// que leer el `PATH`, que ya se hace también.
// 0.1.1
fn atajos() -> Vec<Atajo> {
    let atajo = |nombre: &str, detalle, icono, claves, accion| Atajo {
        nombre: nombre.to_string(),
        detalle,
        icono,
        claves,
        accion,
    };
    let tarjeta = |nombre: &str, detalle, icono, claves, widget| {
        atajo(nombre, detalle, icono, claves, Accion::Emergente(widget))
    };
    // Cada modo con su dibujo: con la paleta en los tres, «Modo claro» y
    // «Modo oscuro» salían seguidos con el mismo icono.
    let tema = |nombre: &str, claves, modo| {
        atajo(
            nombre,
            t("Apariencia del escritorio", "Desktop appearance"),
            match modo {
                crate::tema::ModoTema::Claro => "brillo",
                crate::tema::ModoTema::Oscuro => "noche",
                crate::tema::ModoTema::Automatico => "brillo-automatico",
            },
            claves,
            Accion::Apariencia {
                modo,
                acento: crate::tema::acento_actual(),
            },
        )
    };
    let reducidos = crate::tema::efectos_reducidos();
    let nocturna = crate::luz_nocturna_encendida();
    let despierto = super::mantener_despierto();
    let mut lista = vec![
        atajo(
            t("Bloquear sesión", "Lock session"),
            t("Pedir contraseña para volver", "Require password to unlock"),
            "bloquear",
            &["bloqueo", "lock"],
            Accion::Bloquear,
        ),
        atajo(
            t("Apagar o reiniciar", "Shut down or restart"),
            t("Abrir controles de energía", "Open power controls"),
            "apagar",
            &[
                "apagado",
                "poweroff",
                "reiniciar",
                "restart",
                "suspender",
                "suspend",
                "cerrar sesion",
                "log out",
            ],
            Accion::Emergente("apagar"),
        ),
        atajo(
            if reducidos {
                t("Efectos completos", "Full visual effects")
            } else {
                t("Reducir efectos", "Reduce visual effects")
            },
            if reducidos {
                t(
                    "Devolver el desenfoque y las animaciones",
                    "Restore blur and animations",
                )
            } else {
                t(
                    "Quitar el desenfoque y las animaciones de ventana",
                    "Disable blur and window animations",
                )
            },
            "apariencia",
            &["efectos", "animaciones", "desenfoque", "effects", "animations"],
            Accion::AlternarEfectos,
        ),
        atajo(
            if nocturna {
                t("Apagar luz nocturna", "Turn off night light")
            } else {
                t("Encender luz nocturna", "Turn on night light")
            },
            t("Tono cálido en la pantalla", "Warm screen color temperature"),
            "noche",
            &["luz nocturna", "night", "calido", "warm"],
            Accion::LuzNocturna(!nocturna),
        ),
        atajo(
            if despierto {
                t("Dejar de mantener despierto", "Allow screen sleep")
            } else {
                t("Mantener despierto", "Keep awake")
            },
            t("Que la pantalla no se apague sola", "Prevent display sleep"),
            "mantener",
            &["cafeina", "no dormir", "despierto", "awake", "caffeine"],
            Accion::MantenerDespierto,
        ),
        atajo(
            t("Captura de pantalla", "Screenshot"),
            t("Elegir zona, ventana o pantalla", "Select area, window or screen"),
            "captura",
            &["screenshot", "capturar", "foto"],
            Accion::AbrirCaptura,
        ),
        atajo(
            t("Vista de escritorios", "Desktops overview"),
            t("Todos los escritorios y sus ventanas", "All desktops and open windows"),
            "control",
            &["escritorios", "mission control", "ventanas", "desktops", "overview"],
            Accion::VistaEscritorios,
        ),
        atajo(
            t("Nuevo escritorio", "New desktop"),
            t("Añadir un escritorio vacío", "Add an empty desktop"),
            "vista-anadir",
            &["crear escritorio", "anadir escritorio", "new desktop"],
            Accion::CrearEscritorio,
        ),
        atajo(
            "Launchpad",
            t("Todas las aplicaciones", "All applications"),
            "launchpad",
            &["aplicaciones", "apps"],
            Accion::Launchpad,
        ),
        atajo(
            t("Borrar notificaciones", "Clear notifications"),
            t("Vaciar el centro de notificaciones", "Empty notification center"),
            "papelera",
            &["limpiar notificaciones", "clear notifications"],
            Accion::BorrarNotificaciones,
        ),
        atajo(
            t("Acerca de este equipo", "About this computer"),
            t("Versión de BookOS y hardware", "BookOS version and hardware"),
            "acerca",
            &["sistema", "version", "about"],
            Accion::Acerca,
        ),
        atajo(
            t("Atajos de teclado", "Keyboard shortcuts"),
            t("Todos los atajos y gestos", "All shortcuts and gestures"),
            "teclado",
            &["atajos", "teclas", "shortcuts", "gestos", "ayuda"],
            Accion::Atajos,
        ),
        tarjeta(
            t("Sonido", "Sound"),
            t("Volumen y salida de audio", "Volume and audio output"),
            "volumen-alto",
            &["volumen", "audio", "altavoces", "volume", "sound"],
            "volumen",
        ),
        tarjeta(
            t("Brillo", "Brightness"),
            t("Brillo de la pantalla", "Display brightness"),
            "brillo",
            &["pantalla", "brightness", "display"],
            "brillo",
        ),
        tarjeta(
            t("Wi-Fi y red", "Wi-Fi & network"),
            t("Redes disponibles", "Available networks"),
            "wifi",
            &["wifi", "red", "internet", "network"],
            "red",
        ),
        tarjeta(
            "Bluetooth",
            t("Dispositivos cercanos", "Nearby devices"),
            "bluetooth",
            &["auriculares", "buds", "headphones"],
            "bluetooth",
        ),
        tarjeta(
            t("Energía", "Power"),
            t("Batería y perfil de rendimiento", "Battery and performance profile"),
            "bateria-carga",
            &["energia", "bateria", "power", "battery", "rendimiento"],
            // El widget se llama «bateria»: con «energia», que era lo que
            // mandaba esto antes, el shell no encontraba tarjeta y no abría nada.
            "bateria",
        ),
        tarjeta(
            t("Notificaciones", "Notifications"),
            t("Avisos y no molestar", "Alerts and do not disturb"),
            "notificaciones",
            &["no molestar", "avisos", "do not disturb"],
            "notificaciones",
        ),
        tarjeta(
            t("Calendario", "Calendar"),
            t("Fecha y eventos", "Date and events"),
            "emoji-clock",
            &["fecha", "eventos", "calendar", "date"],
            "reloj",
        ),
        tarjeta(
            t("Centro de control", "Control Center"),
            t("Interruptores rápidos", "Quick toggles"),
            "control",
            &["control"],
            "control",
        ),
        tema(
            t("Modo oscuro", "Dark mode"),
            &["oscuro", "dark", "tema"],
            crate::tema::ModoTema::Oscuro,
        ),
        tema(
            t("Modo claro", "Light mode"),
            &["claro", "light", "tema"],
            crate::tema::ModoTema::Claro,
        ),
        tema(
            t("Tema automático", "Automatic theme"),
            &["automatico", "auto", "tema"],
            crate::tema::ModoTema::Automatico,
        ),
    ];
    // Las páginas de Ajustes, por los ids de BookOS-Settings (`pages` en
    // src/main.js): uno que no esté ahí se ignora en silencio.
    const PAGINAS: &[(&str, &str, &str, &str, &[&str])] = &[
        (
            "Conexiones",
            "Connections",
            "conexiones",
            "wifi",
            &["wifi", "red", "network"],
        ),
        (
            "Pantalla",
            "Display",
            "pantalla",
            "pantalla",
            &["resolucion", "escala", "monitor", "resolution", "scale"],
        ),
        (
            "Sonido",
            "Sound",
            "sonido",
            "volumen-alto",
            &["audio", "volumen", "volume"],
        ),
        (
            "Batería",
            "Battery",
            "bateria",
            "bateria-100",
            &["energia", "bateria", "power"],
        ),
        (
            "Notificaciones",
            "Notifications",
            "notificaciones",
            "notificaciones",
            &[],
        ),
        (
            "Temas",
            "Themes",
            "temas",
            "apariencia",
            &["apariencia", "acento", "colores", "colors"],
        ),
        (
            "Fondos de pantalla",
            "Wallpaper",
            "fondos",
            "fondo",
            &["fondo", "wallpaper"],
        ),
        (
            "Bloqueo de pantalla",
            "Lock screen",
            "bloqueo",
            "bloquear",
            &["bloqueo", "lock"],
        ),
        ("Bluetooth", "Bluetooth", "bluetooth", "bluetooth", &[]),
        (
            "Teclado",
            "Keyboard",
            "teclado",
            "teclado",
            &["distribucion", "idioma", "layout", "language"],
        ),
        (
            "Touchpad y ratón",
            "Touchpad and mouse",
            "touchpadmouse",
            "touchpad",
            &["raton", "gestos", "mouse", "gestures"],
        ),
        (
            "Huella",
            "Fingerprint and password",
            "huella",
            "bloquear",
            &["huella", "dactilar", "fingerprint"],
        ),
        (
            "Cuentas",
            "Accounts",
            "cuentas",
            "acerca",
            &["usuario", "contrasena", "user", "password"],
        ),
        (
            "Aplicaciones",
            "Applications",
            "aplicaciones",
            "launchpad",
            &["predeterminadas", "default"],
        ),
        (
            "Actualizaciones",
            "Software update",
            "actualizacion",
            "reiniciar",
            &["actualizar", "update"],
        ),
        (
            "Accesibilidad",
            "Accessibility",
            "accesibilidad",
            "ojo",
            &["zoom", "contraste", "contrast"],
        ),
        (
            "Seguridad",
            "Security and privacy",
            "seguridad",
            "bloquear",
            &["privacidad", "privacy"],
        ),
        (
            "Fecha, idioma y general",
            "General management",
            "general",
            "preferencias",
            &["fecha", "idioma", "hora", "date", "language", "time"],
        ),
    ];
    lista.extend(PAGINAS.iter().map(|(es, en, id, icono, claves)| {
        atajo(
            &format!(
                "{} {}",
                t("Ajustes de", "Settings:"),
                t(es, en).to_lowercase()
            ),
            t("Abrir en Ajustes", "Open in Settings"),
            icono,
            claves,
            Accion::Lanzar(format!("bookos-settings --page {id}")),
        )
    }));
    lista
}

/// Lo que una acción elegida tiene que cambiar aquí antes de salir hacia el
/// compositor. «Mantener despierto» no lleva el estado nuevo dentro, como
/// sí lo lleva la luz nocturna: lo cambia quien la pide, y desde el buscador
/// no lo cambiaba nadie, así que no hacía nada.
// añadido en la 0.2.0
fn aplicar(accion: Accion) -> Accion {
    if accion == Accion::MantenerDespierto {
        super::alternar_mantener_despierto();
    }
    accion
}

/// Lo mismo que [`apps::puntuar`] para una acción: menor es mejor.
///
/// Cuenta el nombre y las claves, y una consulta de varias palabras encaja si
/// cada palabra empieza alguna de ellas: «luz noc» encuentra la luz nocturna y
/// «ajustes tecl» el teclado. Con menos de dos letras no hay acciones: con una
/// sola encajaría media lista y taparía las aplicaciones.
// 0.1.1
fn puntuar_atajo(atajo: &Atajo, consulta: &str) -> Option<u32> {
    let q = apps::normalizar(consulta);
    if q.chars().count() < 2 {
        return None;
    }
    let nombre = apps::normalizar(&atajo.nombre);
    if nombre == q {
        return Some(0);
    }
    if nombre.starts_with(&q) {
        return Some(1);
    }
    let claves: Vec<String> = atajo.claves.iter().map(|c| apps::normalizar(c)).collect();
    if claves.iter().any(|c| c.starts_with(&q)) {
        return Some(1);
    }
    let palabras: Vec<&str> = nombre
        .split_whitespace()
        .chain(claves.iter().flat_map(|c| c.split_whitespace()))
        .collect();
    if q.split_whitespace()
        .all(|p| palabras.iter().any(|w| w.starts_with(p)))
    {
        return Some(2);
    }
    (q.len() >= 3 && nombre.contains(&q)).then_some(4)
}

/// Evalúa una cuenta: `+ - * / ^ %`, paréntesis, `x` y `×` como por, `÷`, y la
/// coma decimal española. `None` si no es una cuenta —hace falta al menos un
/// operador, o «2024» sería una calculadora— o si no da un número finito.
// 0.1.1
fn calcular(consulta: &str) -> Option<f64> {
    let texto: String = consulta
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            ',' => '.',
            'x' | 'X' | '×' => '*',
            '÷' => '/',
            otro => otro,
        })
        .collect();
    if !texto
        .chars()
        .all(|c| c.is_ascii_digit() || "+-*/^%().".contains(c))
        || !texto.chars().any(|c| c.is_ascii_digit())
        || !texto.chars().skip(1).any(|c| "+-*/^%".contains(c))
    {
        return None;
    }
    let mut cuenta = Cuenta {
        texto: texto.as_bytes(),
        i: 0,
    };
    let valor = cuenta.suma()?;
    (cuenta.i == texto.len() && valor.is_finite()).then_some(valor)
}

/// Descenso recursivo de toda la vida: suma → producto → potencia → factor.
struct Cuenta<'a> {
    texto: &'a [u8],
    i: usize,
}

impl Cuenta<'_> {
    // 0.1.1
    fn ver(&self) -> Option<u8> {
        self.texto.get(self.i).copied()
    }

    // 0.1.1
    fn suma(&mut self) -> Option<f64> {
        let mut valor = self.producto()?;
        while let Some(op @ (b'+' | b'-')) = self.ver() {
            self.i += 1;
            let otro = self.producto()?;
            valor = if op == b'+' {
                valor + otro
            } else {
                valor - otro
            };
        }
        Some(valor)
    }

    // 0.1.1
    fn producto(&mut self) -> Option<f64> {
        let mut valor = self.potencia()?;
        while let Some(op @ (b'*' | b'/' | b'%')) = self.ver() {
            self.i += 1;
            let otro = self.potencia()?;
            valor = match op {
                b'*' => valor * otro,
                b'/' => valor / otro,
                _ => valor % otro,
            };
        }
        Some(valor)
    }

    /// Asocia por la derecha: `2^3^2` es 2⁹, como en cualquier calculadora.
    // 0.1.1
    fn potencia(&mut self) -> Option<f64> {
        let base = self.factor()?;
        if self.ver() == Some(b'^') {
            self.i += 1;
            return Some(base.powf(self.potencia()?));
        }
        Some(base)
    }

    // 0.1.1
    fn factor(&mut self) -> Option<f64> {
        match self.ver()? {
            b'-' => {
                self.i += 1;
                Some(-self.factor()?)
            }
            b'(' => {
                self.i += 1;
                let valor = self.suma()?;
                (self.ver()? == b')').then(|| self.i += 1)?;
                Some(valor)
            }
            _ => {
                let inicio = self.i;
                while self.ver().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.texto[inicio..self.i])
                    .ok()?
                    .parse()
                    .ok()
            }
        }
    }
}

/// Con coma decimal y sin ceros de relleno: `2,5`, no `2.500000`.
// 0.1.1
fn formatear_numero(valor: f64) -> String {
    let texto = format!("{:.10}", valor);
    let texto = texto.trim_end_matches('0').trim_end_matches('.');
    let texto = if texto == "-0" { "0" } else { texto };
    texto.replace('.', ",")
}

/// ¿Existe un ejecutable con ese nombre en el `PATH`?
// 0.1.0
// modificado en la 0.2.0 -- la búsqueda es la de apps, compartida con TryExec
fn en_el_path(orden: &str) -> bool {
    !orden.contains('/') && crate::apps::existe_programa(orden)
}

/// La respuesta a una pregunta de estado, si la consulta es una de ellas.
///
/// Los datos son los mismos que enseña el panel ([`PanelData`]), leídos de
/// sysfs en el momento: preguntar por la batería en el buscador y mirar el
/// icono del panel tienen que decir lo mismo.
// 0.1.0
fn estado(consulta: &str) -> Option<Resultado> {
    let q = apps::normalizar(consulta.trim());
    let quiere = |claves: &[&str]| claves.iter().any(|c| c.starts_with(&q) && q.len() >= 3);

    if quiere(&["bateria", "battery"]) {
        let bat = PanelData::read().battery?;
        let restante = match bat.minutes {
            Some(m) if m >= 60 => format!(", {}h {:02}min", m / 60, m % 60),
            Some(m) => format!(", {m} min"),
            None => String::new(),
        };
        let como = if bat.charging {
            t("cargando", "charging")
        } else if bat.plugged {
            t("enchufada", "plugged in")
        } else {
            t("con batería", "on battery")
        };
        let pct = format!("{} %", bat.percent);
        return Some(Resultado::Dato {
            titulo: format!("{} {pct}", t("Batería al", "Battery at")),
            valor: format!("{como}{restante}"),
            copiar: Some(pct),
            icono: insignia(if bat.charging {
                "bateria-carga"
            } else if bat.percent > 60 {
                "bateria-100"
            } else if bat.percent > 20 {
                "bateria-50"
            } else {
                "bateria-0"
            }),
        });
    }
    if quiere(&["red", "wifi", "internet", "network"]) {
        let red = PanelData::read().network;
        let (titulo, icono) = match red {
            Some(r) if r.up => (
                match r.kind {
                    crate::state::Link::Wifi => t("Wi-Fi conectado", "Wi-Fi connected"),
                    crate::state::Link::Cable => t("Cable conectado", "Wired connected"),
                },
                "wifi",
            ),
            _ => (t("Sin conexión", "Offline"), "sin-red"),
        };
        return Some(Resultado::Dato {
            titulo: titulo.to_string(),
            valor: t("Estado de la red", "Network status").to_string(),
            copiar: Some(titulo.to_string()),
            icono: insignia(icono),
        });
    }
    if quiere(&["hora", "fecha", "reloj", "time", "clock", "date"]) {
        let reloj = PanelData::read().clock;
        return Some(Resultado::Dato {
            titulo: reloj.clone(),
            valor: t("Hora local · Intro copia", "Local time · Enter copies").to_string(),
            copiar: Some(reloj),
            icono: insignia("acerca"),
        });
    }
    if quiere(&["memoria", "ram", "memory"]) {
        let (usada, total) = memoria()?;
        let titulo = format!("{usada:.1} / {total:.1} GiB");
        return Some(Resultado::Dato {
            titulo: format!("{} {}", titulo, t("en uso", "used")),
            valor: format!(
                "{} % {}",
                (usada / total * 100.0).round() as u32,
                t("de la memoria", "of memory")
            ),
            copiar: Some(titulo),
            icono: insignia("cpu"),
        });
    }
    if quiere(&["cpu", "procesador", "carga", "processor"]) {
        let (carga, hilos) = cpu_estado()?;
        let pct = format!("{:.0} %", (carga / hilos as f32 * 100.0).min(999.0));
        return Some(Resultado::Dato {
            titulo: format!("CPU {pct}"),
            valor: format!(
                "{} · {hilos} {}",
                t("carga de 1 minuto", "1-minute load"),
                t("hilos lógicos", "logical threads")
            ),
            copiar: Some(pct),
            icono: insignia("cpu"),
        });
    }
    if quiere(&["gpu", "grafica"]) {
        let uso = gpu_estado()?;
        let pct = format!("{uso} %");
        return Some(Resultado::Dato {
            titulo: format!("GPU {pct}"),
            valor: t("ocupación del motor gráfico", "graphics engine usage").to_string(),
            copiar: Some(pct),
            icono: insignia("control"),
        });
    }
    None
}

/// «brillo 30» o «volumen 50%»: una fila que lo pone al pulsar Intro. Va
/// arriba del todo porque quien escribe un número tras la palabra no está
/// buscando la aplicación que se llame así.
// añadido en la 0.2.0
fn ajuste(consulta: &str) -> Option<Resultado> {
    let (palabra, numero) = consulta.split_once(char::is_whitespace)?;
    let nivel: u8 = numero.trim().trim_end_matches('%').trim().parse().ok()?;
    let nivel = nivel.min(100);
    match apps::normalizar(palabra).as_str() {
        // El mismo suelo que el deslizador y las teclas: a cero no se ve ni la
        // pantalla con la que volver a subirlo.
        "brillo" | "brightness" => Some(Resultado::Accion {
            nombre: format!("{} {} %", t("Brillo al", "Brightness to"), nivel.max(5)),
            detalle: t("Pulsa Intro para ponerlo", "Press Enter to set it").into(),
            icono: insignia("brillo"),
            accion: Accion::Brillo(nivel.max(5)),
        }),
        "volumen" | "volume" | "vol" => Some(Resultado::Accion {
            nombre: format!("{} {nivel} %", t("Volumen al", "Volume to")),
            detalle: t("Pulsa Intro para ponerlo", "Press Enter to set it").into(),
            icono: insignia("volumen-alto"),
            accion: Accion::Volumen(nivel),
        }),
        _ => None,
    }
}

/// Memoria usada y total, en GiB, de `/proc/meminfo`.
///
/// «Usada» es total menos `MemAvailable`, que es lo que dice el kernel que se
/// puede repartir sin ir a swap — no `MemFree`, que en Linux siempre parece
/// poca porque la caché cuenta como ocupada.
// 0.1.0
fn memoria() -> Option<(f32, f32)> {
    let texto = std::fs::read_to_string("/proc/meminfo").ok()?;
    let campo = |nombre: &str| {
        texto
            .lines()
            .find(|l| l.starts_with(nombre))?
            .split_whitespace()
            .nth(1)?
            .parse::<f32>()
            .ok()
    };
    let total = campo("MemTotal:")?;
    let disponible = campo("MemAvailable:")?;
    // Vienen en kibibytes.
    Some(((total - disponible) / 1048576.0, total / 1048576.0))
}

/// Carga media del último minuto y número de hilos lógicos. Es una lectura
/// barata de procfs y no despierta ningún servicio externo.
// 0.1.0
fn cpu_estado() -> Option<(f32, usize)> {
    let texto = std::fs::read_to_string("/proc/loadavg").ok()?;
    let carga = texto.split_whitespace().next()?.parse::<f32>().ok()?;
    let hilos = std::thread::available_parallelism().ok()?.get();
    Some((carga, hilos))
}

/// Porcentaje que expone el driver DRM para las GPU que lo soportan.
///
/// No se inventa una cifra a partir de la carga de la CPU: si el kernel no
/// ofrece `gpu_busy_percent`, el resultado no aparece y el buscador sigue
/// siendo honesto.
// 0.1.0
fn gpu_estado() -> Option<u32> {
    let entradas = std::fs::read_dir("/sys/class/drm").ok()?;
    for entrada in entradas.flatten() {
        let nombre = entrada.file_name();
        let nombre = nombre.to_string_lossy();
        if !nombre.starts_with("card") || nombre.contains('-') {
            continue;
        }
        let Ok(texto) = std::fs::read_to_string(entrada.path().join("device/gpu_busy_percent"))
        else {
            continue;
        };
        if let Ok(valor) = texto.trim().parse::<u32>() {
            return Some(valor.min(100));
        }
    }
    None
}

/// Quita la última palabra y los espacios que la siguen, que es lo que hace
/// Ctrl+Retroceso en GTK y en Qt: «abrir ajustes  » queda en «abrir ».
// 0.1.0
fn borrar_palabra(texto: &mut String) {
    let sin_espacios = texto.trim_end().len();
    let corte = texto[..sin_espacios]
        .rfind(char::is_whitespace)
        .map_or(0, |i| {
            i + texto[i..].chars().next().map_or(1, char::len_utf8)
        });
    texto.truncate(corte);
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_peticion_de_archivo_saca_el_nombre() {
        assert_eq!(
            archivo_pedido("Busca un archivo llamado BookOS-HIG"),
            Some("BookOS-HIG")
        );
        assert_eq!(
            archivo_pedido("abre la carpeta Descargas"),
            Some("Descargas")
        );
        assert_eq!(archivo_pedido("encuentra notas.md"), Some("notas.md"));
        assert_eq!(
            archivo_pedido("busca un archivo de BookOS-HIG"),
            Some("BookOS-HIG")
        );
        assert_eq!(archivo_pedido("pon el brillo al 30"), None);
        assert_eq!(archivo_pedido("busca"), None);
    }

    #[test]
    fn la_regex_escapa_lo_escrito() {
        assert_eq!(escapar_regex("a.b(c)"), "a\\.b\\(c\\)");
    }

    /// «zenith» + espacio abre el ámbito; Retroceso con el campo vacío lo
    /// quita; y Tab sobre la sugerencia entra con el texto puesto.
    #[test]
    fn la_palabra_y_el_tabulador_abren_el_ambito() {
        use crate::TeclaPulsada as T;
        let mut b = Buscador::new((1440.0, 900.0), Vec::new());
        for c in "zenith ".chars() {
            b.tecla(T::Caracter(c));
        }
        assert_eq!(b.ambito, Ambito::Zenith);
        assert!(b.consulta.is_empty());
        b.tecla(T::Retroceso);
        for c in "@zenith ".chars() {
            b.tecla(T::Caracter(c));
        }
        assert_eq!(b.ambito, Ambito::Zenith, "con @ delante también");
        b.tecla(T::Retroceso);
        assert_eq!(b.ambito, Ambito::Todo);

        for c in "pon música tranquila".chars() {
            b.tecla(T::Caracter(c));
        }
        let zenith = b
            .resultados
            .iter()
            .position(|r| {
                matches!(
                    r,
                    Resultado::Ambito {
                        ambito: Ambito::Zenith,
                        ..
                    }
                )
            })
            .expect("sugerencia de Zenith con dos palabras");
        b.seleccion = zenith;
        b.tecla(T::Tabulador);
        assert_eq!(b.ambito, Ambito::Zenith);
        assert_eq!(b.consulta, "pon música tranquila");
        let Tecla::Hacer(accion) = b.tecla(T::Intro) else {
            panic!("Intro sobre Zenith tiene que preguntarle");
        };
        assert_eq!(
            accion,
            Accion::Lanzar("bookos-assistant --preguntar 'pon música tranquila'".into())
        );
    }

    fn buscador_con(consulta: &str) -> Buscador {
        let mut b = Buscador::new((1440.0, 900.0), Vec::new());
        for c in consulta.chars() {
            b.tecla(crate::TeclaPulsada::Caracter(c));
        }
        b
    }

    /// «@» a secas lista a quién hablar; con letras filtra; Intro pone la píldora.
    #[test]
    fn la_arroba_lista_los_ambitos_y_intro_entra_en_uno() {
        let todos = buscador_con("@");
        assert_eq!(todos.resultados.len(), 2);
        assert!(todos.resultados.iter().all(|r| matches!(r, Resultado::Mencion { .. })));
        let mut b = buscador_con("@zen");
        assert_eq!(b.resultados.len(), 1);
        assert!(matches!(
            b.tecla(crate::TeclaPulsada::Intro),
            Tecla::Consumida
        ));
        assert_eq!(b.ambito, Ambito::Zenith);
        assert!(b.consulta.is_empty());
        assert_eq!(buscador_con("@xyz").resultados.len(), 0);
    }

    /// La fila elegida de brillo mide más (lleva la barra), la hit-test lo
    /// cuenta, y ← → dejan un valor pendiente que solo Intro aplica.
    #[test]
    fn la_fila_de_nivel_se_abre_y_lo_pendiente_lo_aplica_intro() {
        let mut b = buscador_con("bril");
        // Sin retroiluminación (CI, portátil sin sysfs) no hay fila: nada que probar.
        let Some(Resultado::Nivel { que, valor, .. }) = b.resultados.first() else {
            return;
        };
        let (que, valor) = (*que, *valor);
        assert_eq!(b.alto_de(0), FILA + BARRA_EXTRA);
        assert_eq!(b.alto_de(1), FILA);
        // La segunda fila empieza donde acaba la ampliada.
        let y1 = b.y_lista() + FILA + BARRA_EXTRA + 1.0;
        assert_eq!(b.fila_en(100.0, y1), Some(1));
        assert!(matches!(b.tecla(crate::TeclaPulsada::Intro), Tecla::Consumida));
        b.tecla(crate::TeclaPulsada::Izquierda);
        assert_eq!(b.pendiente, Some((que, valor.saturating_sub(5).max(5))));
        match b.tecla(crate::TeclaPulsada::Intro) {
            Tecla::Hacer(a) => assert_eq!(a, que.accion(valor.saturating_sub(5).max(5))),
            _ => panic!("Intro con algo pendiente tiene que aplicarlo"),
        }
        // Al elegir otra fila la barra se cierra y lo pendiente se olvida.
        let mut c = buscador_con("bril");
        c.tecla(crate::TeclaPulsada::Derecha);
        c.tecla(crate::TeclaPulsada::Abajo);
        assert_eq!(c.pendiente, None);
        assert_eq!(c.alto_de(0), FILA);
    }

    #[test]
    fn brillo_y_volumen_con_numero_dan_un_ajuste() {
        let accion = |consulta| match ajuste(consulta) {
            Some(Resultado::Accion { accion, .. }) => Some(accion),
            _ => None,
        };
        assert_eq!(accion("brillo 30"), Some(Accion::Brillo(30)));
        assert_eq!(accion("Brillo 30%"), Some(Accion::Brillo(30)));
        assert_eq!(accion("brightness 70"), Some(Accion::Brillo(70)));
        // El suelo del deslizador y el techo del tanto por ciento.
        assert_eq!(accion("brillo 0"), Some(Accion::Brillo(5)));
        assert_eq!(accion("volumen 250"), Some(Accion::Volumen(100)));
        assert_eq!(accion("vol 0"), Some(Accion::Volumen(0)));
        assert_eq!(accion("brillo"), None);
        assert_eq!(accion("brillo alto"), None);
        assert_eq!(accion("firefox 30"), None);
    }

    #[test]
    fn ctrl_retroceso_borra_la_ultima_palabra() {
        let casos = [
            ("abrir ajustes", "abrir "),
            ("abrir ajustes  ", "abrir "),
            ("ajustes", ""),
            ("   ", ""),
            ("", ""),
            ("música año", "música "),
        ];
        for (antes, despues) in casos {
            let mut t = antes.to_string();
            borrar_palabra(&mut t);
            assert_eq!(t, despues, "desde «{antes}»");
        }
    }

    #[test]
    fn la_paleta_compartida_resuelve_los_roles_de_ambos_temas() {
        let json = r##"{
            "light": {"card":"#FFFFFF","tx":"#241E1A","tx2":"#78675B",
                "div":"rgba(149,78,0,0.14)","brd":"rgba(149,78,0,0.12)",
                "hov":"rgba(149,78,0,0.07)","blue":"#954E00"},
            "dark": {"card":"#241E1A","tx":"#FAF2EC","tx2":"#B4A295",
                "div":"rgba(255,179,123,0.13)","brd":"rgba(255,179,123,0.1)",
                "hov":"rgba(255,179,123,0.09)","blue":"#FFB37B"}
        }"##;
        let claro = Colores::desde_json(json, true).expect("roles claros publicados por Ajustes");
        let oscuro =
            Colores::desde_json(json, false).expect("roles oscuros publicados por Ajustes");
        assert_eq!(claro.tarjeta, Color::WHITE);
        assert_eq!(oscuro.tarjeta, tema::hex(0x241e1a));
        assert_eq!(oscuro.tarjeta.a, 1.0, "el matiz no necesita translucidez");
        assert_eq!(claro.texto, tema::hex(0x241e1a));
        assert_eq!(oscuro.secundario, tema::hex(0xb4a295));
        assert_eq!(oscuro.acento, tema::hex(0xffb37b));
        assert_eq!(claro.divisor, tema::hexa(0x954e00, 0.14));
        assert_eq!(oscuro.borde, tema::hexa(0xffb37b, 0.1));
        assert_eq!(oscuro.hover, tema::hexa(0xffb37b, 0.09));
    }

    #[test]
    fn una_paleta_incompleta_o_invalida_deja_el_respaldo() {
        for json in ["", "{}", r##"{"dark":{"card":"#241E1A"}}"##] {
            assert!(Colores::desde_json(json, false).is_none());
        }
        for color in ["#zzzzzz", "#fff", "rgba(256,0,0,1)", "rgba(0,0,0,NaN)"] {
            assert!(tema::color_css(color).is_none(), "color inválido: {color}");
        }
    }

    fn app(nombre: &str, exec: &str) -> apps::App {
        apps::App {
            nombre: nombre.to_string(),
            detalle: None,
            exec: exec.to_string(),
            icono: String::new(),
            normalizado: nombre.to_lowercase(),
        }
    }

    #[test]
    fn el_detalle_de_una_app_no_ensena_el_exec_crudo() {
        let mut a = app("Navegador", "flatpak run --branch=stable org.mozilla.firefox");
        let sin_desc = resultado_app(&a, None);
        assert!(!sin_desc.detalle().contains("flatpak run"));
        a.detalle = Some("Navega por la web".into());
        let con_desc = resultado_app(&a, None);
        assert_eq!(con_desc.detalle(), "Navega por la web");
    }

    #[test]
    fn cursor_movible_y_pegado_limpio() {
        use crate::TeclaPulsada as T;
        let mut b = Buscador::new((1440.0, 900.0), Vec::new());
        for c in "ac".chars() {
            b.tecla(T::Caracter(c));
        }
        b.tecla(T::Izquierda);
        b.tecla(T::Caracter('b'));
        assert_eq!(b.consulta, "abc");
        b.tecla(T::Inicio);
        b.tecla(T::Suprimir);
        assert_eq!(b.consulta, "bc");
        b.tecla(T::Fin);
        b.tecla(T::Pegar("d\ne\tf\x07".into()));
        assert_eq!(b.consulta, "bcd e f");
    }

    #[test]
    fn calculo_se_copia_con_intro_y_muestra_confirmacion() {
        use crate::TeclaPulsada as T;
        let mut b = buscador_con("2+3*4");
        let Tecla::Hacer(accion) = b.tecla(T::Intro) else {
            panic!("Intro sobre una cuenta tiene que copiar el resultado");
        };
        assert_eq!(accion, Accion::CopiarTexto("14".into()));
        assert!(b.confirmando());
    }

    #[test]
    fn una_app_que_casa_gana_al_binario_del_path() {
        let apps = [app("Sh", "mi-sh")];
        let lista = resultados(&apps, &Default::default(), &[], "sh");
        assert!(matches!(&lista[0], Resultado::App { nombre, .. } if nombre == "Sh"));
    }

    #[test]
    fn ctrl_a_y_escribir_sustituye_la_consulta() {
        use crate::TeclaPulsada as T;
        let mut b = Buscador::new((1920.0, 1080.0), Vec::new());
        for c in "firefox".chars() {
            b.tecla(T::Caracter(c));
        }
        b.tecla(T::SeleccionarTodo);
        b.tecla(T::Caracter('k'));
        assert_eq!(b.consulta, "k");
        b.tecla(T::SeleccionarTodo);
        b.tecla(T::Retroceso);
        assert!(b.consulta.is_empty());
        // Una flecha deshace la selección sin tocar el texto.
        b.tecla(T::Caracter('x'));
        b.tecla(T::SeleccionarTodo);
        b.tecla(T::Abajo);
        b.tecla(T::Caracter('y'));
        assert_eq!(b.consulta, "xy");
    }

    #[test]
    fn sin_consulta_no_hay_lista() {
        assert!(resultados(&[app("Konsole", "konsole")], &Default::default(), &[], "").is_empty());
    }

    #[test]
    fn el_prefijo_manda_ejecutar_sin_buscar_aplicaciones() {
        let apps = [app("Konsole", "konsole")];
        let lista = resultados(&apps, &Default::default(), &[], "> ls -la");
        assert_eq!(lista.len(), 1, "con `>` solo hay orden");
        assert!(matches!(&lista[0], Resultado::Comando(o) if o == "ls -la"));
    }

    #[test]
    fn una_aplicacion_se_encuentra_por_su_nombre() {
        let apps = [app("Konsole", "konsole"), app("Kate", "kate")];
        let lista = resultados(&apps, &Default::default(), &[], "kon");
        assert!(
            matches!(&lista[0], Resultado::App { nombre, .. } if nombre == "Konsole"),
            "la que empieza por lo escrito va primera"
        );
    }

    /// `sh` existe en cualquier Linux, así que esto no depende de la máquina.
    #[test]
    fn un_ejecutable_del_path_se_ofrece_como_comando() {
        let lista = resultados(&[], &Default::default(), &[], "sh");
        assert!(
            lista
                .iter()
                .any(|r| matches!(r, Resultado::Comando(o) if o == "sh")),
            "un binario del PATH tiene que poder lanzarse sin el prefijo"
        );
    }

    #[test]
    fn una_ruta_no_se_busca_en_el_path() {
        assert!(
            !en_el_path("/usr/bin/sh"),
            "con barra no es un nombre suelto"
        );
    }

    #[test]
    fn los_comandos_del_escritorio_son_acciones_controladas() {
        let lista = resultados(&[], &Default::default(), &[], t("bloquear", "lock"));
        assert!(matches!(
            lista.first(),
            Some(Resultado::Accion {
                accion: Accion::Bloquear,
                ..
            })
        ));
    }

    /// «efectos» y «animaciones» llevan al mismo sitio: quien quiere quitar el
    /// adorno no tiene por qué saber cómo lo llamamos aquí.
    #[test]
    fn los_efectos_se_encuentran_por_lo_que_quitan() {
        for consulta in ["efectos", "animaciones", "desenfoque"] {
            let lista = resultados(&[], &Default::default(), &[], consulta);
            assert!(
                matches!(
                    lista.first(),
                    Some(Resultado::Accion {
                        accion: Accion::AlternarEfectos,
                        ..
                    })
                ),
                "«{consulta}» no encuentra el interruptor de efectos"
            );
        }
    }

    #[test]
    fn con_el_campo_vacio_sugiere_lo_mas_usado() {
        let apps = [
            app("Kate", "kate"),
            app("Konsole", "konsole"),
            app("Gimp", "gimp"),
        ];
        let usos =
            std::collections::HashMap::from([("kate".to_string(), 2), ("konsole".to_string(), 9)]);
        let nombres: Vec<_> = resultados(&apps, &usos, &[], "")
            .iter()
            .map(|r| r.nombre().to_string())
            .collect();
        assert_eq!(nombres, ["Konsole", "Kate"], "sin usos no se sugiere");
    }

    #[test]
    fn entre_dos_que_encajan_igual_va_la_mas_usada() {
        let apps = [app("Kate", "kate"), app("Konsole", "konsole")];
        let usos = std::collections::HashMap::from([("konsole".to_string(), 5)]);
        let lista = resultados(&apps, &usos, &[], "k");
        assert_eq!(lista[0].nombre(), "Konsole");
    }

    #[test]
    fn las_acciones_se_encuentran_por_cualquier_palabra() {
        for (consulta, buscada) in [
            (t("luz noc", "night li"), t("luz nocturna", "night light")),
            (t("oscuro", "dark"), t("Modo oscuro", "Dark mode")),
            (
                t("ajustes tecl", "settings keyb"),
                t("Ajustes de teclado", "Settings: keyboard"),
            ),
            (t("volumen", "volume"), t("Sonido", "Sound")),
            (
                t("captura", "screenshot"),
                t("Captura de pantalla", "Screenshot"),
            ),
        ] {
            let lista = resultados(&[], &Default::default(), &[], consulta);
            assert!(
                lista.iter().any(|r| r.nombre().contains(buscada)),
                "«{consulta}» no encuentra «{buscada}»"
            );
        }
    }

    #[test]
    fn energia_abre_la_tarjeta_de_la_bateria() {
        let lista = resultados(&[], &Default::default(), &[], t("energía", "power"));
        assert!(lista.iter().any(|r| matches!(
            r,
            Resultado::Accion {
                accion: Accion::Emergente("bateria"),
                ..
            }
        )));
    }

    /// Una aplicación abierta va delante y lleva a su ventana, no a otra nueva.
    #[test]
    fn una_app_abierta_va_delante_y_se_muestra() {
        let apps = [app("Kate", "kate"), app("Konsole", "konsole")];
        let abiertas = [("konsole".to_string(), "org.kde.konsole".to_string())];
        let lista = resultados(&apps, &Default::default(), &abiertas, "k");
        assert_eq!(lista[0].nombre(), "Konsole");
        assert_eq!(
            lista[0].accion(),
            Some(Accion::Mostrar {
                app_id: "org.kde.konsole".into(),
                exec: "konsole".into()
            })
        );
        assert_eq!(lista[1].accion(), Some(Accion::Lanzar("kate".into())));
    }

    #[test]
    fn calcula_cuentas_con_coma_y_precedencia() {
        assert_eq!(calcular("2+3*4"), Some(14.0));
        assert_eq!(calcular("(2+3)*4"), Some(20.0));
        assert_eq!(calcular("1,5 x 2"), Some(3.0));
        assert_eq!(calcular("2^3^2"), Some(512.0));
        assert_eq!(calcular("-3+1"), Some(-2.0));
        assert_eq!(formatear_numero(10.0 / 4.0), "2,5");
        assert_eq!(formatear_numero(1.0 / 3.0), "0,3333333333");
    }

    #[test]
    fn un_numero_suelto_o_un_texto_no_son_cuentas() {
        for consulta in ["2024", "firefox", "1/0", "2+", "(1+2", "-5"] {
            assert_eq!(calcular(consulta), None, "«{consulta}»");
        }
    }

    #[test]
    fn el_ancho_se_adapta_a_la_pantalla_logica() {
        assert_eq!(ancho_para(800.0), ANCHO_MIN);
        assert_eq!(ancho_para(1920.0), ANCHO);
        assert_eq!(ancho_para(3840.0), ANCHO_MAX);
    }

    #[test]
    fn confirmando_deja_de_animar_cuando_el_sello_asienta_y_programa_alarma() {
        let mut b = Buscador::new((1920.0, 1080.0), Vec::new());
        b.iniciar_confirmacion("Brillo al 50 %".into());
        assert!(b.confirmando());
        let alarma = b.proxima_alarma().expect("debe programar cierre único");
        assert!(alarma <= CONFIRMACION && alarma > std::time::Duration::from_millis(900));

        // Al asentarse el sello, la espera restante no pide repintados CPU a 60/120 Hz.
        b.sello.fijar(1.0);
        b.elegida_y.fijar(b.elegida_y.valor());
        b.alto.fijar(b.alto.valor());
        assert!(!b.animando());
        assert!(b.confirmando());
        assert!(b.proxima_alarma().is_some());
    }

    #[test]
    fn confirmando_oculta_el_resto_y_bloquea_nueva_interaccion() {
        use crate::TeclaPulsada as T;
        let mut b = buscador_con("2+3*4");
        b.agarrada = true;
        b.pendiente = Some((Que::Brillo, 55));
        let Tecla::Hacer(_) = b.tecla(T::Intro) else {
            panic!("Intro sobre una cuenta tiene que confirmar");
        };
        assert!(b.confirmando());
        assert!(!b.agarrado());
        assert_eq!(b.pendiente, None);
        assert_eq!(b.alto_objetivo(), FILA + MARGEN * 2.0);
        // No se puede escribir, mover, pulsar ni volver a confirmar.
        assert!(matches!(b.tecla(T::Caracter('x')), Tecla::Consumida));
        assert!(matches!(b.tecla(T::Intro), Tecla::Consumida));
        assert!(matches!(b.tecla(T::Escape), Tecla::Consumida));
        assert_eq!(b.consulta, "2+3*4");
        assert!(!b.puntero(Some((20.0, b.y_lista() + 10.0))));
        assert_eq!(b.pulsar(20.0, b.y_lista() + 10.0), None);
        assert!(!b.arrastre_en(20.0, b.y_lista() + FILA + 5.0));
        let nodos = b.accesible();
        assert_eq!(nodos.len(), 1);
        assert_eq!(nodos[0].rol, accesskit::Role::Status);
    }

    #[test]
    fn tecla_control_con_todo_seleccionado_no_borra_la_consulta() {
        use crate::TeclaPulsada as T;
        let mut b = buscador_con("hola");
        b.todo_seleccionado = true;
        b.tecla(T::Caracter('\x07'));
        assert_eq!(b.consulta, "hola");
        assert!(b.todo_seleccionado);
    }

    #[test]
    fn mover_cursor_resiste_indice_desalineado() {
        let mut b = buscador_con("añ");
        b.cursor = 2; // dentro del carácter 'ñ'
        b.mover_cursor_izquierda();
        assert_eq!(b.cursor, 0);
        b.cursor = 999;
        b.mover_cursor_derecha();
        assert_eq!(b.cursor, b.consulta.len());
    }
}
