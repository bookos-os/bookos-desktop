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
use iced_widget::{Space, column, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, PanelData, apps};

use super::{Ancla, Tecla};

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
    fn actuales() -> Self {
        let respaldo = Self {
            tarjeta: tema::card(),
            texto: tema::texto(),
            secundario: tema::TEXTO2,
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

    fn desde_json(json: &str, claro: bool) -> Option<Self> {
        let paleta: serde_json::Value = serde_json::from_str(json).ok()?;
        let roles = paleta.get(if claro { "light" } else { "dark" })?;
        let rol = |nombre| color_css(roles.get(nombre)?.as_str()?);
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

fn color_css(valor: &str) -> Option<Color> {
    if let Some(hex) = valor.strip_prefix('#') {
        return (hex.len() == 6)
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
            .map(tema::hex);
    }
    let rgba = valor.strip_prefix("rgba(")?.strip_suffix(')')?;
    let mut canales = rgba.split(',').map(str::trim);
    let mut canal = || canales.next()?.parse::<u8>().ok().map(|n| n as f32 / 255.0);
    let (r, g, b) = (canal()?, canal()?, canal()?);
    let a = canales.next()?.parse::<f32>().ok()?;
    if canales.next().is_some() || !(0.0..=1.0).contains(&a) {
        return None;
    }
    Some(Color::from_rgba(r, g, b, a))
}

/// Lo que se puede encontrar.
pub enum Resultado {
    /// Una aplicación instalada.
    App {
        nombre: String,
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
    /// Una respuesta que se lee aquí mismo: no hay nada que lanzar.
    Dato {
        titulo: String,
        valor: String,
        icono: Option<Icono>,
    },
}

impl Resultado {
    fn nombre(&self) -> &str {
        match self {
            Self::App { nombre, .. } => nombre,
            Self::Comando(orden) => orden,
            Self::Accion { nombre, .. } => nombre,
            Self::Dato { titulo, .. } => titulo,
        }
    }

    /// El renglón pequeño de debajo: qué es esto y qué va a pasar al pulsarlo.
    fn detalle(&self) -> String {
        match self {
            Self::App {
                abierta: Some(_), ..
            } => "Abierta · ir a la ventana".to_string(),
            Self::App { exec, .. } => exec.clone(),
            Self::Comando(_) => "Ejecutar orden".to_string(),
            Self::Accion { detalle, .. } => detalle.clone(),
            Self::Dato { valor, .. } => valor.clone(),
        }
    }

    fn icono(&self) -> Option<&Icono> {
        match self {
            Self::App { icono, .. } | Self::Dato { icono, .. } => icono.as_ref(),
            Self::Comando(_) => None,
            Self::Accion { icono, .. } => icono.as_ref(),
        }
    }

    /// Qué hacer al elegirlo. Un dato no lanza nada: ya ha contestado.
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
            Self::Dato { .. } => None,
        }
    }
}

pub struct Buscador {
    /// Todo el layout se expresa en píxeles lógicos. El compositor rasteriza
    /// después a físicos con la escala del monitor, evitando que HiDPI cambie
    /// el tamaño aparente o desplace el hit-test.
    ancho: f32,
    consulta: String,
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
    señalada: Option<usize>,
    /// Tras Ctrl+A: lo siguiente que se escriba o borre sustituye la consulta
    /// entera. No hay cursor movible, así que basta con esto y no un rango.
    todo_seleccionado: bool,
}

impl Buscador {
    pub fn new(pantalla: (f32, f32), abiertas: Vec<(String, String)>) -> Self {
        crate::icono::olvidar_ausentes();
        let apps = apps::leer();
        let mut buscador = Self {
            ancho: ancho_para(pantalla.0),
            consulta: String::new(),
            apps,
            usos: crate::config::cargar_usos(),
            abiertas,
            resultados: Vec::new(),
            seleccion: 0,
            señalada: None,
            todo_seleccionado: false,
        };
        buscador.buscar();
        buscador
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    pub fn size(&self) -> (f32, f32) {
        (self.ancho, Self::alto_para(self.resultados.len()))
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
    pub fn alto_colocacion() -> f32 {
        CAMPO
    }

    /// Sin velo, como Spotlight: el buscador es algo que se consulta de paso,
    /// y oscurecer la pantalla lo convertía en un modal que tapa lo que estás
    /// haciendo —que a menudo es justo de donde sale lo que vas a escribir—.
    pub fn velo(&self) -> iced_core::Color {
        iced_core::Color::TRANSPARENT
    }

    /// Rehace la lista con lo que hay escrito.
    fn buscar(&mut self) {
        self.resultados = resultados(&self.apps, &self.usos, &self.abiertas, &self.consulta);
        self.seleccion = 0;
        self.señalada = None;
    }

    /// Dónde empieza la lista dentro de la tarjeta. Tiene que incluir todo el
    /// aire que apila `view()` entre el campo y los resultados.
    fn y_lista(&self) -> f32 {
        CAMPO + DIVISOR + SEPARACION
    }

    /// Qué fila cae en `y`, si cae en alguna.
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho {
            return None;
        }
        let dentro = y - self.y_lista();
        if dentro < 0.0 {
            return None;
        }
        let i = (dentro / FILA) as usize;
        (i < self.resultados.len().min(MAXIMO)).then_some(i)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.fila_en(x, y));
        if nueva == self.señalada {
            return false;
        }
        self.señalada = nueva;
        true
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.fila_en(x, y)?;
        self.resultados.get(i)?.accion()
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        let visibles = self.resultados.len().min(MAXIMO);
        if std::mem::take(&mut self.todo_seleccionado)
            && matches!(tecla, T::Caracter(_) | T::Retroceso | T::BorrarTodo)
        {
            self.consulta.clear();
            if let T::Caracter(c) = tecla {
                self.consulta.push(c);
            }
            self.buscar();
            return Tecla::Consumida;
        }
        match tecla {
            T::SeleccionarTodo => {
                self.todo_seleccionado = !self.consulta.is_empty();
                Tecla::Consumida
            }
            // Esc borra lo escrito antes de cerrar, como el launchpad.
            T::Escape if !self.consulta.is_empty() => {
                self.consulta.clear();
                self.buscar();
                Tecla::Consumida
            }
            T::Escape => Tecla::Cerrar,
            T::Caracter(c) => {
                self.consulta.push(c);
                self.buscar();
                Tecla::Consumida
            }
            T::Retroceso => {
                self.consulta.pop();
                self.buscar();
                Tecla::Consumida
            }
            // Ctrl+Retroceso borra la última palabra, como en cualquier campo
            // de texto. Antes caía en `Ignorada` y el compositor se la pasaba
            // a la ventana con foco, que borraba una palabra suya por detrás.
            T::BorrarTodo => {
                borrar_palabra(&mut self.consulta);
                self.buscar();
                Tecla::Consumida
            }
            T::Abajo if visibles > 0 => {
                self.seleccion = (self.seleccion + 1) % visibles;
                self.señalada = None;
                Tecla::Consumida
            }
            T::Arriba if visibles > 0 => {
                self.seleccion = (self.seleccion + visibles - 1) % visibles;
                self.señalada = None;
                Tecla::Consumida
            }
            T::Intro => match self.resultados.get(self.seleccion).and_then(|r| r.accion()) {
                Some(accion) => Tecla::Hacer(accion),
                // Un dato ya ha contestado: Intro sobre él no cierra la
                // ventana, porque lo que quieres es seguir leyéndolo.
                None => Tecla::Consumida,
            },
            // El buscador es modal: lo que no entiende tampoco es para la
            // ventana de debajo, que no se ve y no sabe que se escribe aquí.
            _ => Tecla::Consumida,
        }
    }

    pub fn animando(&self) -> bool {
        false
    }

    pub fn view(&self) -> PanelElement<'_> {
        let colores = Colores::actuales();
        let lupa = icono::propio("buscar")
            .map(|ic| icono::ver_teñido_propio(&ic, 26.0, colores.secundario))
            .unwrap_or_else(crate::widget::vacio);
        // El cursor va pegado al texto y siempre a la vista: no parpadea. Un
        // parpadeo son dos repintados por segundo de una tarjeta de 640 px
        // mientras no haces nada, y este compositor no despierta para eso.
        let escrito: PanelElement<'_> = if self.consulta.is_empty() {
            text("Buscar apps, ajustes o acciones")
                .size(TEXTO_CAMPO)
                .color(colores.secundario)
                .into()
        } else if self.todo_seleccionado {
            let acento = colores.acento;
            container(
                text(&self.consulta)
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
            text(format!("{}▏", self.consulta))
                .size(TEXTO_CAMPO)
                .color(colores.texto)
                .into()
        };
        let campo = container(
            row![lupa, Space::new().width(Length::Fixed(12.0)), escrito,].align_y(Vertical::Center),
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
            contenido = contenido.push(lista);
            // El respiro bajo la última fila: sin él su realce llega al borde y
            // se come la esquina redondeada de la tarjeta. Va aquí y no como
            // alto de más en `size()`, que es donde estaba: allí eran píxeles
            // transparentes fuera de la tarjeta y descuadraban el centrado.
            contenido = contenido.push(Space::new().height(Length::Fixed(MARGEN)));
        }

        container(contenido)
            .width(Length::Fixed(self.ancho))
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

    fn fila<'a>(
        &'a self,
        i: usize,
        resultado: &'a Resultado,
        colores: Colores,
    ) -> PanelElement<'a> {
        let elegida = i == self.seleccion;
        let señalada = self.señalada == Some(i);
        // El acento se reserva para lo que Intro va a lanzar. El ratón por
        // encima solo levanta un velo de tinta: si las dos cosas se pintaran
        // igual, pasar el ratón parecería cambiar lo que va a ejecutarse.
        let fondo = match (elegida, señalada) {
            (true, _) => tema::alfa(colores.acento, 0.10),
            (false, true) => colores.hover,
            (false, false) => iced_core::Color::TRANSPARENT,
        };
        let dibujo: PanelElement<'a> = match resultado.icono() {
            // Los de aplicación no se tiñen —el zorro de Firefox en silueta
            // blanca no lo quiere nadie—; los propios sí, que son monocromos.
            Some(ic) if matches!(resultado, Resultado::App { .. }) => icono::ver(ic, ICONO, ICONO),
            Some(ic @ Icono::Raster(_)) => icono::ver(ic, ICONO, ICONO),
            Some(ic) => icono::ver_teñido(ic, ICONO, Some(colores.texto)),
            None => icono::propio("terminal")
                .map(|ic| icono::ver_teñido_propio(&ic, ICONO, colores.texto))
                .unwrap_or_else(crate::widget::vacio),
        };
        let textos = column![
            text(super::recortar_texto(
                resultado.nombre(),
                self.ancho - 140.0,
                15.0
            ))
            .size(15.0)
            .color(colores.texto),
            text(super::recortar_texto(
                &resultado.detalle(),
                self.ancho - 140.0,
                12.0
            ))
            .size(12.0)
            .color(colores.secundario),
        ];
        // El realce va **dentro** de un margen y con esquinas: pegado a los
        // bordes de la tarjeta, la última fila se comía su esquina redondeada.
        let fila = container(
            row![
                Space::new().width(Length::Fixed(18.0 - MARGEN)),
                dibujo,
                Space::new().width(Length::Fixed(14.0)),
                textos,
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
        });
        container(fila)
            .width(Length::Fixed(self.ancho))
            .padding([0, MARGEN as u16])
            .into()
    }
}

/// Calcula el ancho del buscador en coordenadas lógicas.
///
/// La pantalla física no entra aquí: `ShellHost` ya la ha dividido por la
/// escala del monitor. Por eso un 4K a 200 % y un 4K a 100 % se comportan como
/// dos escritorios lógicos distintos, que es exactamente lo que espera el
/// usuario. El límite superior conserva la lectura de una fila y evita que el
/// buscador se convierta en una barra gigantesca en un ultrawide.
fn ancho_para(pantalla_logica: f32) -> f32 {
    (pantalla_logica * ANCHO / 1920.0).clamp(ANCHO_MIN, ANCHO_MAX)
}

/// La lista de resultados para una consulta. Suelta para poder probarla sin
/// leer los `.desktop` de la máquina.
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
    if let Some(valor) = calcular(consulta) {
        lista.push(Resultado::Dato {
            titulo: format!("= {}", formatear_numero(valor)),
            valor: format!("Resultado de {consulta}"),
            icono: icono::propio("control"),
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
                icono: icono::propio(atajo.icono),
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
        if primera_palabra == consulta {
            lista.insert(0, comando);
        } else {
            lista.push(comando);
        }
    }
    lista
}

fn resultado_app(app: &apps::App, abierta: Option<String>) -> Resultado {
    Resultado::App {
        nombre: app.nombre.clone(),
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
    let tema = |nombre: &str, claves, modo| {
        atajo(
            nombre,
            "Apariencia del escritorio",
            "apariencia",
            claves,
            Accion::Apariencia {
                modo,
                acento: crate::tema::acento_actual(),
            },
        )
    };
    let reducidos = crate::tema::efectos_reducidos();
    let nocturna = crate::luz_nocturna_encendida();
    let mut lista = vec![
        atajo(
            "Bloquear sesión",
            "Pedir contraseña para volver",
            "bloquear",
            &["bloqueo", "lock"],
            Accion::Bloquear,
        ),
        atajo(
            "Apagar o reiniciar",
            "Abrir controles de energía",
            "apagar",
            &[
                "apagado",
                "poweroff",
                "reiniciar",
                "suspender",
                "cerrar sesion",
            ],
            Accion::Emergente("apagar"),
        ),
        atajo(
            if reducidos {
                "Efectos completos"
            } else {
                "Reducir efectos"
            },
            if reducidos {
                "Devolver el desenfoque y las animaciones"
            } else {
                "Quitar el desenfoque y las animaciones de ventana"
            },
            "apariencia",
            &["efectos", "animaciones", "desenfoque"],
            Accion::AlternarEfectos,
        ),
        atajo(
            if nocturna {
                "Apagar luz nocturna"
            } else {
                "Encender luz nocturna"
            },
            "Tono cálido en la pantalla",
            "noche",
            &["luz nocturna", "night", "calido"],
            Accion::LuzNocturna(!nocturna),
        ),
        atajo(
            "Mantener despierto",
            "Que la pantalla no se apague sola",
            "mantener",
            &["cafeina", "no dormir", "despierto"],
            Accion::MantenerDespierto,
        ),
        atajo(
            "Captura de pantalla",
            "Elegir zona, ventana o pantalla",
            "captura",
            &["screenshot", "capturar", "foto"],
            Accion::AbrirCaptura,
        ),
        atajo(
            "Vista de escritorios",
            "Todos los escritorios y sus ventanas",
            "control",
            &["escritorios", "mission control", "ventanas"],
            Accion::VistaEscritorios,
        ),
        atajo(
            "Nuevo escritorio",
            "Añadir un escritorio vacío",
            "vista-anadir",
            &["crear escritorio", "anadir escritorio"],
            Accion::CrearEscritorio,
        ),
        atajo(
            "Launchpad",
            "Todas las aplicaciones",
            "launchpad",
            &["aplicaciones", "apps"],
            Accion::Launchpad,
        ),
        atajo(
            "Borrar notificaciones",
            "Vaciar el centro de notificaciones",
            "papelera",
            &["limpiar notificaciones"],
            Accion::BorrarNotificaciones,
        ),
        atajo(
            "Acerca de este equipo",
            "Versión de BookOS y hardware",
            "acerca",
            &["sistema", "version", "about"],
            Accion::Acerca,
        ),
        tarjeta(
            "Sonido",
            "Volumen y salida de audio",
            "volumen-alto",
            &["volumen", "audio", "altavoces"],
            "volumen",
        ),
        tarjeta(
            "Brillo",
            "Brillo de la pantalla",
            "brillo",
            &["pantalla"],
            "brillo",
        ),
        tarjeta(
            "Wi-Fi y red",
            "Redes disponibles",
            "wifi",
            &["wifi", "red", "internet"],
            "red",
        ),
        tarjeta(
            "Bluetooth",
            "Dispositivos cercanos",
            "bluetooth",
            &["auriculares", "buds"],
            "bluetooth",
        ),
        tarjeta(
            "Energía",
            "Batería y perfil de rendimiento",
            "bateria-carga",
            &["energia", "bateria", "power", "rendimiento"],
            // El widget se llama «bateria»: con «energia», que era lo que
            // mandaba esto antes, el shell no encontraba tarjeta y no abría nada.
            "bateria",
        ),
        tarjeta(
            "Notificaciones",
            "Avisos y no molestar",
            "notificaciones",
            &["no molestar", "avisos"],
            "notificaciones",
        ),
        tarjeta(
            "Calendario",
            "Fecha y eventos",
            "emoji-clock",
            &["fecha", "eventos"],
            "reloj",
        ),
        tarjeta(
            "Centro de control",
            "Interruptores rápidos",
            "control",
            &["control"],
            "control",
        ),
        tema(
            "Modo oscuro",
            &["oscuro", "dark", "tema"],
            crate::tema::ModoTema::Oscuro,
        ),
        tema(
            "Modo claro",
            &["claro", "light", "tema"],
            crate::tema::ModoTema::Claro,
        ),
        tema(
            "Tema automático",
            &["automatico", "auto", "tema"],
            crate::tema::ModoTema::Automatico,
        ),
    ];
    // Las páginas de Ajustes, por los ids de BookOS-Settings (`pages` en
    // src/main.js): uno que no esté ahí se ignora en silencio.
    const PAGINAS: &[(&str, &str, &str, &[&str])] = &[
        ("Conexiones", "conexiones", "wifi", &["wifi", "red"]),
        (
            "Pantalla",
            "pantalla",
            "pantalla",
            &["resolucion", "escala", "monitor"],
        ),
        ("Sonido", "sonido", "volumen-alto", &["audio", "volumen"]),
        ("Batería", "bateria", "bateria-100", &["energia", "bateria"]),
        ("Notificaciones", "notificaciones", "notificaciones", &[]),
        (
            "Temas",
            "temas",
            "apariencia",
            &["apariencia", "acento", "colores"],
        ),
        (
            "Fondos de pantalla",
            "fondos",
            "fondo",
            &["fondo", "wallpaper"],
        ),
        ("Bloqueo de pantalla", "bloqueo", "bloquear", &["bloqueo"]),
        ("Bluetooth", "bluetooth", "bluetooth", &[]),
        ("Teclado", "teclado", "teclado", &["distribucion", "idioma"]),
        (
            "Touchpad y ratón",
            "touchpadmouse",
            "touchpad",
            &["raton", "gestos", "mouse"],
        ),
        ("Huella", "huella", "bloquear", &["huella", "dactilar"]),
        ("Cuentas", "cuentas", "acerca", &["usuario", "contrasena"]),
        (
            "Aplicaciones",
            "aplicaciones",
            "launchpad",
            &["predeterminadas"],
        ),
        (
            "Actualizaciones",
            "actualizacion",
            "reiniciar",
            &["actualizar", "update"],
        ),
        (
            "Accesibilidad",
            "accesibilidad",
            "ojo",
            &["zoom", "contraste"],
        ),
        ("Seguridad", "seguridad", "bloquear", &["privacidad"]),
        (
            "Fecha, idioma y general",
            "general",
            "preferencias",
            &["fecha", "idioma", "hora"],
        ),
    ];
    lista.extend(PAGINAS.iter().map(|(nombre, id, icono, claves)| {
        atajo(
            &format!("Ajustes de {}", nombre.to_lowercase()),
            "Abrir en Ajustes",
            icono,
            claves,
            Accion::Lanzar(format!("bookos-settings --page {id}")),
        )
    }));
    lista
}

/// Lo mismo que [`apps::puntuar`] para una acción: menor es mejor.
///
/// Cuenta el nombre y las claves, y una consulta de varias palabras encaja si
/// cada palabra empieza alguna de ellas: «luz noc» encuentra la luz nocturna y
/// «ajustes tecl» el teclado. Con menos de dos letras no hay acciones: con una
/// sola encajaría media lista y taparía las aplicaciones.
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
    fn ver(&self) -> Option<u8> {
        self.texto.get(self.i).copied()
    }

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
    fn potencia(&mut self) -> Option<f64> {
        let base = self.factor()?;
        if self.ver() == Some(b'^') {
            self.i += 1;
            return Some(base.powf(self.potencia()?));
        }
        Some(base)
    }

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
fn formatear_numero(valor: f64) -> String {
    let texto = format!("{:.10}", valor);
    let texto = texto.trim_end_matches('0').trim_end_matches('.');
    let texto = if texto == "-0" { "0" } else { texto };
    texto.replace('.', ",")
}

/// ¿Existe un ejecutable con ese nombre en el `PATH`?
fn en_el_path(orden: &str) -> bool {
    if orden.is_empty() || orden.contains('/') {
        return false;
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(orden).is_file())
}

/// La respuesta a una pregunta de estado, si la consulta es una de ellas.
///
/// Los datos son los mismos que enseña el panel ([`PanelData`]), leídos de
/// sysfs en el momento: preguntar por la batería en el buscador y mirar el
/// icono del panel tienen que decir lo mismo.
fn estado(consulta: &str) -> Option<Resultado> {
    let q = consulta.to_lowercase();
    let quiere = |claves: &[&str]| claves.iter().any(|c| c.starts_with(&q) && q.len() >= 3);

    if quiere(&["bateria", "batería", "battery"]) {
        let bat = PanelData::read().battery?;
        let restante = match bat.minutes {
            Some(m) if m >= 60 => format!(", {}h {:02}min", m / 60, m % 60),
            Some(m) => format!(", {m} min"),
            None => String::new(),
        };
        let como = if bat.charging {
            "cargando"
        } else if bat.plugged {
            "enchufada"
        } else {
            "con batería"
        };
        return Some(Resultado::Dato {
            titulo: format!("Batería al {} %", bat.percent),
            valor: format!("{como}{restante}"),
            icono: icono::propio(if bat.charging {
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
    if quiere(&["red", "wifi", "internet"]) {
        let red = PanelData::read().network;
        let (titulo, icono) = match red {
            Some(r) if r.up => (
                match r.kind {
                    crate::state::Link::Wifi => "Wi-Fi conectado",
                    crate::state::Link::Cable => "Cable conectado",
                },
                "wifi",
            ),
            _ => ("Sin conexión", "sin-red"),
        };
        return Some(Resultado::Dato {
            titulo: titulo.to_string(),
            valor: "Estado de la red".to_string(),
            icono: icono::propio(icono),
        });
    }
    if quiere(&["brillo", "pantalla"]) {
        let brillo = PanelData::read().brightness?;
        return Some(Resultado::Dato {
            titulo: format!("Brillo al {brillo} %"),
            valor: "Retroiluminación de la pantalla".to_string(),
            icono: icono::propio("brillo"),
        });
    }
    if quiere(&["hora", "fecha", "reloj"]) {
        return Some(Resultado::Dato {
            titulo: PanelData::read().clock,
            valor: "Hora local".to_string(),
            icono: icono::propio("acerca"),
        });
    }
    if quiere(&["memoria", "ram"]) {
        let (usada, total) = memoria()?;
        return Some(Resultado::Dato {
            titulo: format!("{:.1} de {:.1} GiB en uso", usada, total),
            valor: format!("{} % de la memoria", (usada / total * 100.0).round() as u32),
            icono: icono::propio("cpu"),
        });
    }
    if quiere(&["cpu", "procesador", "carga"]) {
        let (carga, hilos) = cpu_estado()?;
        return Some(Resultado::Dato {
            titulo: format!("CPU al {:.0} %", (carga / hilos as f32 * 100.0).min(999.0)),
            valor: format!("carga de 1 minuto · {hilos} hilos lógicos"),
            icono: icono::propio("cpu"),
        });
    }
    if quiere(&["gpu", "grafica", "gráfica"]) {
        let uso = gpu_estado()?;
        return Some(Resultado::Dato {
            titulo: format!("GPU al {uso} %"),
            valor: "ocupación del motor gráfico".to_string(),
            icono: icono::propio("control"),
        });
    }
    None
}

/// Memoria usada y total, en GiB, de `/proc/meminfo`.
///
/// «Usada» es total menos `MemAvailable`, que es lo que dice el kernel que se
/// puede repartir sin ir a swap — no `MemFree`, que en Linux siempre parece
/// poca porque la caché cuenta como ocupada.
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
            assert!(color_css(color).is_none(), "color inválido: {color}");
        }
    }

    fn app(nombre: &str, exec: &str) -> apps::App {
        apps::App {
            nombre: nombre.to_string(),
            exec: exec.to_string(),
            icono: String::new(),
            normalizado: nombre.to_lowercase(),
        }
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
        let lista = resultados(&[], &Default::default(), &[], "bloquear");
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
            ("luz noc", "luz nocturna"),
            ("oscuro", "Modo oscuro"),
            ("ajustes tecl", "Ajustes de teclado"),
            ("volumen", "Sonido"),
            ("captura", "Captura de pantalla"),
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
        let lista = resultados(&[], &Default::default(), &[], "energía");
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
}
