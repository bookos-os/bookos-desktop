//! El launchpad: la rejilla de aplicaciones a pantalla completa.
//!
//! Es la versión propia del plasmoide `bookos-launchpad`: rejilla de hasta
//! **7×5** —la forma sale del hueco, ver [`Launchpad::forma`]—, campo de búsqueda arriba del todo, puntos de página abajo, y la misma
//! puntuación de búsqueda (exacto, prefijo, prefijo de palabra, iniciales,
//! substring, subsecuencia difusa).
//!
//! # La rejilla
//!
//! **El paso es cuadrado.** El 0,72 de la pantalla se reparte tomando el eje
//! que menos sitio deja, y no cada eje por su cuenta: aplicado a lo ancho y a
//! lo alto por separado, a 1440×900 la celda salía 173×130 y el icono se
//! quedaba en 57 px con 116 de aire a los lados, con lo que las filas se leían
//! como bandas sueltas. Con el paso cuadrado el icono sube a 76 px.
//!
//! El campo de búsqueda es un input del sistema de diseño —radio 14, §2.3— y
//! su borde solo se vuelve de acento cuando hay algo escrito: el campo no se
//! puede desenfocar, y un borde de acento permanente hace que el azul deje de
//! significar «esto está activo», que es lo único que §2.1 le deja significar.
//!
//! # Carpetas
//!
//! Una carpeta es un nombre, un color y una lista de aplicaciones, y se guarda
//! en `~/.config/bookos/launchpad.conf` —una línea por carpeta— en vez de en el
//! `foldersJson` del plasmoide: leer JSON aquí sería una dependencia nueva para
//! guardar dos listas de nombres.
//!
//! **Las carpetas van primero y las aplicaciones sueltas después.** Las
//! carpetas, por orden alfabético; las sueltas, en el orden en que el usuario
//! las haya colocado arrastrándolas —soltar en el hueco de una celda, no sobre
//! el icono, que eso crea carpeta— y el resto detrás, alfabético. Lo instalado
//! después de colocar nada cae al final, así que el orden guardado nunca
//! decide por una aplicación que no conoce.
//!
//! Abierta, una carpeta es un **panel** —un diálogo: radio 26, sombra de modal,
//! teñido de su color— con su propia rejilla de 4×3. Antes reusaba la de 6×5
//! entera, y ocho aplicaciones repartidas en treinta huecos no se leen como un
//! grupo. Su tapa en la rejilla enseña cuatro miniaturas en 2×2 y cuenta el
//! resto: con las placas del pack de iconos de BookOS, que traen su propio
//! fondo, las nueve de antes eran nueve manchas y dos carpetas distintas se
//! veían iguales a un metro de la pantalla.
//!
//! El color se elige en un selector que nace **cerrado** —un círculo con el
//! color puesto— y se despliega al pulsarlo: diez puntos permanentes en la
//! cabecera pesan más que el nombre de la carpeta, que es lo que hay que leer
//! ahí. Dentro están los diez acentos de Apariencia y una tira de tono para
//! quien quiera el suyo; de la tira sale el **tono** y nada más, porque la
//! saturación y la luminosidad las pone [`de_tono`] y son las que mantienen el
//! contraste que el resto de la paleta da por hecho.
//!
//! Buscando **no hay carpetas**: se busca entre todas las aplicaciones, estén
//! dentro de una o no. Quien escribe «kate» quiere Kate, no que le recuerden
//! dónde lo guardó.
//!
//! # Lo que cuesta
//!
//! Medido en release a 2074x1549: dibujarlo entero **11 ms**, y **32 ms** la
//! primera vez que aparecen iconos que no estaban —resvg tiene que rasterizar
//! los SVG nuevos, y la caché de iced solo guarda los del último fotograma—.
//! Recorrerlo con el ratón cuesta **cero**: el realce va en su propia
//! superficie. Filtrar por texto, 0,09 ms; lo caro nunca es la búsqueda.
//!
//! Por eso **aquí dentro no se anima nada**: hacerlo en el buffer es rasterizar
//! la rejilla entera en cada fotograma. Lo que sí sale barato es rasterizar dos
//! veces —el estado de antes y el de después— y dejar que el compositor cruce
//! las dos superficies en la GPU, que es lo que ya hacía el cambio de página.
//! Los cambios de vista se anuncian con [`Transicion`] y el compositor pone la
//! curva y la duración de §2.7; el movimiento es solo escala y opacidad, que es
//! lo único que el sistema de diseño permite animar.
//!
//! # Lo que **no** está, y por qué
//!
//! - **El fondo de pantalla borroso.** El plasmoide saca el wallpaper de la
//!   configuración de plasmashell y lo desenfoca llamando a `magick` por un
//!   `DataSource` de tipo "executable". Aquí no hay plasmashell de quien
//!   heredarlo, y lanzar ImageMagick desde el compositor para pintar un fondo
//!   es exactamente lo que este proceso no debe hacer. El desenfoque lo hace el
//!   compositor y encima va el velo, teñido del tono del fondo elegido —que
//!   sale del SVG de la familia, sin decodificar el PNG—: ver
//!   [`Launchpad::velo`].
//!
use std::collections::HashMap;

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, image as iced_image, row, svg, text};

use crate::Accion;
use crate::apps::{self, App};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, Tecla};

/// Tope de columnas: con más, la búsqueda visual por fila se alarga y el
/// paso horizontal se encoge por debajo del vertical en pantallas 16:9.
const COLUMNAS_MAX: usize = 7;
/// La fila más baja con la que se sigue usando una quinta fila. Por debajo se
/// pasa a cuatro: a 1440×900 con el dock delante quedan 541 px de rejilla, que
/// en cinco filas dan 108 por fila y un icono de 63 —«se ve pequeño»—; en
/// cuatro, 135 y un icono de 78.
const FILA_MINIMA: f32 = 120.0;

/// La unidad de rejilla de Plasma, de la que salen todos los espaciados del
/// plasmoide. Con la fuente por defecto son 18 px.
const UNIDAD: f32 = 18.0;
/// Alto de la barra de búsqueda: 3 unidades, como el plasmoide.
const BUSQUEDA: f32 = UNIDAD * 3.0;
/// Ancho de la barra: 20 unidades.
const BUSQUEDA_ANCHO: f32 = UNIDAD * 20.0;
/// Separación entre la barra de búsqueda y la rejilla. Con 2 unidades la
/// primera fila quedaba pegada al campo y se leían como un solo bloque; con 4
/// la búsqueda queda aparte, como cabecera. El bloque va centrado, así que
/// esto sube el campo la mitad y baja la rejilla y los puntos la otra mitad.
const HUECO: f32 = UNIDAD * 4.0;
/// Alto de la fila de puntos de página, con su margen.
const PUNTOS: f32 = UNIDAD * 3.0;
/// Aire mínimo entre el launchpad y los bordes de su hueco: el de la pantalla
/// arriba y el del dock abajo.
const AIRE: f32 = UNIDAD * 2.0;
/// Alto reservado a la etiqueta bajo cada icono: 2 unidades.
const ETIQUETA: f32 = UNIDAD * 2.0;
/// Qué fracción de la pantalla ocupa la rejilla. Es el 0,72 del plasmoide: el
/// resto es aire, y ese aire es la mitad del aspecto.
const FRACCION: f32 = 0.72;
/// Lado de cada punto de color del selector de la carpeta abierta, y su paso.
/// El punto es el radio-circle de 18 px que el sistema de diseño le da al
/// selector de un popover (§4).
const COLOR_PUNTO: f32 = 18.0;
const COLOR_PASO: f32 = 28.0;
/// La ranura de cada punto: el punto más el aire que necesita el anillo del
/// elegido, que va **fuera** y separado. Pegado al punto no se distingue de un
/// borde del propio color.
const COLOR_SLOT: f32 = COLOR_PUNTO + 6.0;
/// El círculo cerrado que enseña el color puesto y abre el selector.
const CHIP_COLOR: f32 = 26.0;
/// Márgenes del popover del selector y alto de su tira de tono libre.
const PALETA_PAD: f32 = 10.0;
const TONO_ANCHO: f32 = 10.0 * COLOR_PASO - (COLOR_PASO - COLOR_SLOT);
const TONO_ALTO: f32 = 16.0;
/// Cuántos trozos tiene la tira de tono. Treinta y seis —uno cada diez
/// grados— es donde el degradado deja de verse a bandas a este ancho, y son
/// treinta y seis rectángulos, no un shader que aquí no hay.
const TONO_TROZOS: usize = 36;
/// Cuántos tonos de la tira recorre el teclado. Ver `colores_del_teclado`.
const TONOS_TECLADO: usize = 6;
/// Lado de la ✕ que quita una aplicación, en la esquina de su icono.
const CIERRE: f32 = 22.0;

/// Cuántas miniaturas caben en la tapa de una carpeta.
///
/// Cuatro, en 2×2, y no las nueve de antes: con las placas del pack de iconos
/// —que traen su propio fondo— nueve miniaturas de 20 px son nueve manchas y
/// dos carpetas distintas se ven iguales a un metro de la pantalla. Con más de
/// cuatro dentro se pintan tres y el resto va contado.
const MINIATURAS: usize = 4;

/// La rejilla de dentro de una carpeta: 4×3 y no las 6×5 de fuera.
///
/// Una carpeta tiene menos cosas, y darles el mismo paso que a la rejilla
/// grande dejaba ocho aplicaciones repartidas en treinta huecos, que no se leen
/// como un grupo.
const COLUMNAS_CARPETA: usize = 4;
const FILAS_CARPETA: usize = 3;

/// Márgenes del panel de la carpeta abierta y alto de su cabecera —el nombre,
/// la cuenta y el selector de color—.
const PANEL_PAD: f32 = UNIDAD * 1.5;
const PANEL_CABECERA: f32 = UNIDAD * 3.0;

/// Lo que ocupa una celda de la rejilla: una aplicación o una carpeta.
///
/// Los dos son índices —a `apps` y a `carpetas`— y no datos: la celda se
/// reconstruye en cada `view` y copiar aquí el nombre y el icono sería
/// duplicar lo que ya está en su sitio.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    App(usize),
    Carpeta(usize),
}

/// Un icono que se está arrastrando por la rejilla.
///
/// Guarda **dónde empezó** y no qué se agarró: la lista visible puede
/// reordenarse mientras tanto —crear una carpeta la reordena— y un índice
/// guardado apuntaría a otra cosa. Lo que se agarró se resuelve al soltar.
#[derive(Clone, Copy)]
struct Arrastre {
    /// Posición en `visibles` de lo que se agarró.
    origen: usize,
    inicio: (f32, f32),
    actual: (f32, f32),
    /// Si ya se ha movido lo bastante como para ser un arrastre y no un clic.
    movido: bool,
}

/// Cuánto hay que mover el ratón para que deje de ser un clic, en lógicos.
///
/// Doce y no dos: pulsar un icono con el ratón siempre lo mueve un poco, y con
/// un umbral pequeño lanzar una aplicación se convertía en arrastrarla.
pub const UMBRAL_ARRASTRE: f32 = 12.0;

/// El color de una carpeta: uno de los diez de Apariencia, o uno libre.
///
/// Los diez siguen siendo lo que se ofrece primero, por lo mismo que el acento
/// del sistema: son pares calibrados para leerse en claro y en oscuro. El
/// libre existe porque se pidió, y para que no se salga del contraste que el
/// resto da por hecho **no es un `#rrggbb` cualquiera**: se elige un tono en la
/// tira y la saturación y la luminosidad las pone [`de_tono`], las mismas para
/// todos. Lo que el usuario controla es el tono; lo que hace legible el color,
/// no.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ColorCarpeta {
    Acento(tema::Acento),
    /// Un tono libre en grados del círculo cromático.
    Tono(f32),
}

impl ColorCarpeta {
    pub fn color(self) -> Color {
        match self {
            ColorCarpeta::Acento(a) => a.color(),
            ColorCarpeta::Tono(grados) => de_tono(grados),
        }
    }

    /// Cómo se escribe en `launchpad.conf`: el nombre del acento, o el tono en
    /// grados con una `t` delante. No es un `#rrggbb` porque lo que se guarda
    /// es el tono, no el color: el par claro/oscuro lo decide el tema al
    /// dibujar, igual que con los acentos.
    fn guardado(self) -> String {
        match self {
            ColorCarpeta::Acento(a) => a.nombre().to_string(),
            ColorCarpeta::Tono(grados) => format!("t{}", grados.round() as i32),
        }
    }

    fn desde_guardado(texto: &str) -> Option<Self> {
        if let Some(grados) = texto.strip_prefix('t') {
            return grados.parse::<f32>().ok().map(ColorCarpeta::Tono);
        }
        tema::Acento::desde_nombre(texto).map(ColorCarpeta::Acento)
    }
}

/// Un color de la tira de tono libre, en grados.
///
/// Saturación y luminosidad fijas, y distintas por tema: son las del par
/// calibrado del que salen los acentos —el azul es `#007aff` en claro y
/// `#5c95ff` en oscuro—, así que un tono libre cae en la misma familia que los
/// diez de la tabla en vez de en cualquier sitio.
fn de_tono(grados: f32) -> Color {
    let (s, l): (f32, f32) = if tema::es_claro() {
        (1.0, 0.50)
    } else {
        (1.0, 0.68)
    };
    let h = grados.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::from_rgb(r + m, g + m, b + m)
}

/// El tono dominante del fondo de pantalla elegido, como color.
///
/// Sale del **SVG** de la familia, que es lo que `fondos::tono` ya sabe leer sin
/// decodificar el PNG de 2880×1800. Aquí no se puede hacer lo que hace Ajustes
/// —sacar el color dominante con ImageMagick—: esto corre dentro del proceso
/// del compositor y lanzar procesos para pintar un velo es exactamente lo que
/// no debe hacer.
fn tinte_del_fondo() -> Option<Color> {
    let elegida = crate::fondos::elegida();
    let familias = crate::fondos::instaladas(tema::es_claro());
    let familia = match elegida {
        Some(nombre) => familias.iter().find(|f| f.nombre == nombre)?,
        None => familias
            .iter()
            .find(|f| f.nombre == crate::fondos::POR_DEFECTO)
            .or(familias.first())?,
    };
    crate::fondos::tono(familia).map(de_tono)
}

/// Un cambio de vista del launchpad que **el compositor tiene que animar**.
///
/// Aquí dentro no se puede animar nada: rasterizar la rejilla cuesta 11 ms
/// medidos, así que hacerlo por fotograma es exactamente lo que el módulo lleva
/// evitando desde el principio. Lo que sí sale barato es rasterizar **dos
/// veces** —el estado de antes y el de después— y dejar que el compositor
/// cruce las dos superficies en la GPU, que es lo que ya hace con el cambio de
/// página. Esto es el aviso de «acabo de cambiar de estado, guarda lo que había
/// y crúzalo así».
pub use super::Transicion;

/// Un grupo de aplicaciones con su nombre.
#[derive(Clone, Debug)]
pub struct Carpeta {
    pub nombre: String,
    /// Su color. `None` = el gris de siempre.
    pub color: Option<ColorCarpeta>,
    /// Los `exec` de sus aplicaciones, en el orden en que se metieron.
    ///
    /// Se guardan por `exec` y no por índice en `apps` porque el índice cambia
    /// al instalar o quitar programas, y esto sobrevive entre sesiones.
    pub apps: Vec<String>,
}

pub struct Launchpad {
    /// Tamaño **lógico** del hueco donde va: la pantalla entera, menos el
    /// dock si se ve por delante. La rejilla se dimensiona a partir de ahí,
    /// como el plasmoide.
    pantalla: (f32, f32),
    apps: Vec<App>,
    /// Las carpetas, tal y como están en el disco.
    carpetas: Vec<Carpeta>,
    /// Los `exec` de las aplicaciones que el usuario ha quitado de la rejilla.
    ///
    /// **Quitadas, no desinstaladas.** El escritorio no borra programas del
    /// sistema: eso es del gestor de paquetes, necesita permisos que aquí no
    /// hay, y una ✕ que desinstala de verdad es la clase de botón que se pulsa
    /// sin querer una vez y se lamenta el resto de la tarde. Se vuelven a ver
    /// borrando su nombre de `launchpad.conf`.
    ocultas: Vec<String>,
    /// Los `exec` de las sueltas colocadas a mano. Ver [`Launchpad::mover_a`].
    orden: Vec<String>,
    /// Lanzamientos por `exec`, para que buscar «vi» ponga delante la que más
    /// se usa. Se lee al abrir: el launchpad se crea de nuevo cada vez.
    usos: HashMap<String, u32>,
    /// El modo de edición: con él, cada icono lleva su ✕ para quitarlo. Se
    /// entra con el botón derecho, como el menú contextual del dock.
    editando: bool,
    /// La carpeta abierta, si se está viendo una por dentro.
    dentro: Option<usize>,
    /// Qué color del selector tiene el foco del teclado, si se abrió con
    /// Tabulador. Con el ratón no hay foco: se pulsa y ya.
    foco_paleta: Option<usize>,
    /// Si el selector de color de la carpeta abierta está desplegado.
    ///
    /// Cerrado es un círculo con el color puesto y nada más: diez puntos
    /// permanentes en la cabecera del panel pesan más que el nombre de la
    /// carpeta, que es lo que hay que leer ahí.
    paleta_abierta: bool,
    /// El nombre que se está escribiendo, si se está renombrando la carpeta
    /// abierta. Una carpeta nace llamándose «Carpeta» y hay que poder
    /// cambiarlo sin editar el fichero a mano.
    renombrando: Option<String>,
    /// Lo que se enseña ahora mismo, ya filtrado y ordenado.
    visibles: Vec<Item>,
    consulta: String,
    pagina: usize,
    /// Posición **dentro de `visibles`** de lo que está seleccionado.
    seleccion: Option<usize>,
    /// Desplazamiento acumulado del touchpad desde el último cambio de página.
    acumulado: f32,
    /// Si el gesto en curso ya pasó su página, cuándo llegó su último evento.
    /// Ver [`Launchpad::desplazar`].
    gesto_gastado: Option<std::time::Instant>,
    /// Cuándo pasó de página el icono arrastrado contra el borde. `None`
    /// fuera del borde. Ver [`Launchpad::puntero`].
    paso_por_borde: Option<std::time::Instant>,
    /// El icono que se está arrastrando, si hay alguno.
    arrastre: Option<Arrastre>,
    /// Dónde está el dock en la pantalla, si se está viendo debajo.
    ///
    /// El launchpad ocupa la pantalla entera, así que sus coordenadas y las de
    /// la pantalla son las mismas y el rectángulo llega tal cual. Se necesita
    /// para dos cosas: no agarrar iconos de la rejilla al pulsar ahí, y saber
    /// que soltar encima significa anclar.
    zona_dock: Option<iced_core::Rectangle>,
    /// El cambio de vista que el compositor todavía no se ha llevado. Ver
    /// [`Transicion`].
    transicion: Option<Transicion>,
    /// El tono dominante del fondo de pantalla, si se pudo saber. Tiñe el
    /// velo; ver [`Launchpad::velo`].
    ///
    /// Se calcula **una vez, al abrir**: leerlo cuesta un `read_dir` y 4 KiB de
    /// SVG, y el velo se pregunta en cada fotograma. El launchpad se construye
    /// cada vez que se abre, así que cambiar de fondo en Ajustes se nota a la
    /// siguiente.
    tinte_fondo: Option<Color>,
    /// Iconos ya resueltos, por índice en `apps`. El `None` de dentro es "se
    /// buscó y no está", que no es lo mismo que "todavía no se ha buscado" —sin
    /// esa distinción se repetiría la búsqueda fallida en cada repintado, que
    /// es justo la cara.
    iconos: HashMap<usize, Option<Icono>>,
}

impl Launchpad {
    pub fn new(pantalla: (f32, f32)) -> Self {
        crate::icono::olvidar_ausentes();
        let apps = apps::leer();
        let guardado = crate::config::cargar_launchpad();
        let carpetas: Vec<Carpeta> = guardado
            .carpetas
            .into_iter()
            .map(|(nombre, color, apps)| Carpeta {
                nombre,
                color: color.as_deref().and_then(ColorCarpeta::desde_guardado),
                apps,
            })
            .collect();
        let ocultas = guardado.ocultas;
        let orden = guardado.orden;
        tracing::debug!(
            n = apps.len(),
            carpetas = carpetas.len(),
            "aplicaciones leídas"
        );
        let mut lp = Self {
            pantalla,
            apps,
            carpetas,
            ocultas,
            orden,
            usos: crate::config::cargar_usos(),
            editando: false,
            dentro: None,
            paleta_abierta: false,
            foco_paleta: None,
            renombrando: None,
            visibles: Vec::new(),
            consulta: String::new(),
            pagina: 0,
            seleccion: None,
            acumulado: 0.0,
            gesto_gastado: None,
            paso_por_borde: None,
            arrastre: None,
            zona_dock: None,
            transicion: None,
            tinte_fondo: tinte_del_fondo(),
            iconos: HashMap::new(),
        };
        lp.rehacer_visibles();
        lp.preparar_pagina();
        lp
    }

    /// Rehace la lista de lo que se enseña, según dónde se esté.
    ///
    /// Es el único sitio donde se decide qué va en la rejilla, y por eso no hay
    /// dos ordenaciones que puedan discrepar: dentro de una carpeta, lo suyo;
    /// buscando, aplicaciones sueltas puntuadas; y en la vista normal, las
    /// carpetas primero y detrás lo que no está en ninguna.
    fn rehacer_visibles(&mut self) {
        if let Some(c) = self.dentro {
            self.visibles = self.apps_de(c).into_iter().map(Item::App).collect();
            return;
        }
        if !self.consulta.is_empty() {
            return;
        }
        let en_carpetas: std::collections::HashSet<&str> = self
            .carpetas
            .iter()
            .flat_map(|c| c.apps.iter().map(String::as_str))
            .collect();
        let ocultas: std::collections::HashSet<&str> =
            self.ocultas.iter().map(String::as_str).collect();
        let mut sueltas: Vec<usize> = (0..self.apps.len())
            .filter(|&i| {
                let exec = self.apps[i].exec.as_str();
                !en_carpetas.contains(exec) && !ocultas.contains(exec)
            })
            .collect();
        // Estable: las que no están en `orden` conservan el alfabético de
        // `apps` detrás de las colocadas.
        sueltas.sort_by_key(|&i| {
            self.orden
                .iter()
                .position(|e| *e == self.apps[i].exec)
                .unwrap_or(usize::MAX)
        });
        let mut visibles: Vec<Item> = (0..self.carpetas.len()).map(Item::Carpeta).collect();
        visibles.extend(sueltas.into_iter().map(Item::App));
        self.visibles = visibles;
    }

    /// Los índices de las aplicaciones de una carpeta, en su orden.
    ///
    /// Las que ya no estén instaladas simplemente no salen: desinstalar algo no
    /// puede dejar un hueco que al pulsarlo no haga nada.
    fn apps_de(&self, carpeta: usize) -> Vec<usize> {
        let Some(c) = self.carpetas.get(carpeta) else {
            return Vec::new();
        };
        c.apps
            .iter()
            .filter_map(|exec| self.apps.iter().position(|a| &a.exec == exec))
            .collect()
    }

    /// Se lleva el cambio de vista pendiente, si lo hay. Lo llama el
    /// compositor justo antes de repintar: quien se lo lleva es quien tiene la
    /// superficie de antes.
    pub fn tomar_transicion(&mut self) -> Option<Transicion> {
        self.transicion.take()
    }

    /// Apunta un cambio de vista para el compositor. El fundido no pisa a una
    /// transición de carpeta: entrar en una carpeta cambia además el color del
    /// panel, y el fundido de después se comería el gesto.
    fn animar(&mut self, transicion: Transicion) {
        if self.transicion.is_some() && transicion == Transicion::Fundido {
            return;
        }
        self.transicion = Some(transicion);
    }

    /// ¿Se está viendo una carpeta por dentro?
    /// Le dice dónde queda el dock, o que no se está viendo.
    pub fn poner_zona_dock(&mut self, zona: Option<iced_core::Rectangle>) {
        self.zona_dock = zona;
    }

    /// El icono que se está arrastrando: dónde va y de qué tamaño, en
    /// coordenadas del launchpad. `None` si no hay arrastre o si todavía no ha
    /// pasado el umbral, que es lo que separa un clic de un arrastre.
    ///
    /// Va **centrado en el cursor** y no anclado por donde se agarró: la mano
    /// apunta con la punta del puntero y esperar que el icono la siga por su
    /// centro es lo que hacen macOS y One UI.
    pub fn rect_agarrado(&self) -> Option<iced_core::Rectangle> {
        let a = self.arrastre.filter(|a| a.movido)?;
        let px = self.icono_px();
        Some(iced_core::Rectangle {
            x: a.actual.0 - px / 2.0,
            y: a.actual.1 - px / 2.0,
            width: px,
            height: px,
        })
    }

    /// El icono agarrado, para pintarlo en su propia superficie.
    ///
    /// Aparte de la rejilla porque moverlo tiene que ser gratis: repintar el
    /// launchpad entero cuesta 11 ms medidos, y a cada movimiento del ratón eso
    /// es un arrastre a tirones. Como superficie propia solo cambia de sitio.
    pub fn vista_agarrado(&self) -> Option<PanelElement<'_>> {
        let a = self.arrastre.filter(|a| a.movido)?;
        let px = self.icono_px();
        match *self.visibles.get(a.origen)? {
            // Al 85 %: el icono va «en el aire» y tiene que verse que no está
            // posado en su celda.
            Item::App(app) => Some(self.baldosa_app(app, px, 0.85)),
            Item::Carpeta(c) => Some(self.tapa_carpeta(c, px, 0.85)),
        }
    }

    /// ¿Cae el punto sobre el dock que se ve debajo?
    fn sobre_el_dock(&self, x: f32, y: f32) -> bool {
        self.zona_dock
            .is_some_and(|r| r.contains(iced_core::Point::new(x, y)))
    }

    pub fn en_carpeta(&self) -> bool {
        self.dentro.is_some()
    }

    /// Sale de la carpeta abierta. `true` si había alguna.
    pub fn salir_de_carpeta(&mut self) -> bool {
        if self.dentro.take().is_none() {
            return false;
        }
        self.animar(Transicion::CerrarCarpeta);
        self.renombrando = None;
        self.paleta_abierta = false;
        self.consulta.clear();
        self.pagina = 0;
        self.seleccion = None;
        self.rehacer_visibles();
        self.preparar_pagina();
        true
    }

    /// El campo de búsqueda, y la ✕ que lo limpia cuando hay algo escrito.
    ///
    /// La ✕ mide 24 y no los 14 del dibujo: el glifo es la señal, pero lo que
    /// se pulsa tiene que caber debajo de un dedo.
    fn rect_busqueda(&self) -> iced_core::Rectangle {
        iced_core::Rectangle {
            x: (self.size().0 - BUSQUEDA_ANCHO) / 2.0,
            y: self.margen_y(),
            width: BUSQUEDA_ANCHO,
            height: BUSQUEDA,
        }
    }

    fn rect_limpiar(&self) -> Option<iced_core::Rectangle> {
        if self.consulta.is_empty() || self.dentro.is_some() {
            return None;
        }
        let campo = self.rect_busqueda();
        Some(iced_core::Rectangle {
            x: campo.x + campo.width - 16.0 - 24.0,
            y: campo.y + (campo.height - 24.0) / 2.0,
            width: 24.0,
            height: 24.0,
        })
    }

    /// El círculo que enseña el color de la carpeta abierta y despliega el
    /// selector. Va a la derecha de su cabecera, donde no compite con el
    /// nombre.
    fn rect_chip_color(&self) -> iced_core::Rectangle {
        let panel = self.rect_panel();
        iced_core::Rectangle {
            x: panel.x + panel.width - PANEL_PAD - CHIP_COLOR,
            y: panel.y + PANEL_PAD + (PANEL_CABECERA - CHIP_COLOR) / 2.0,
            width: CHIP_COLOR,
            height: CHIP_COLOR,
        }
    }

    /// El popover del selector, cuando está desplegado: los diez acentos en
    /// fila y debajo la tira de tono libre.
    fn rect_paleta(&self) -> iced_core::Rectangle {
        let chip = self.rect_chip_color();
        let ancho = TONO_ANCHO + PALETA_PAD * 2.0;
        iced_core::Rectangle {
            // Alineado por la derecha con el chip: crece hacia dentro del
            // panel, que es donde hay sitio.
            x: (chip.x + CHIP_COLOR - ancho).max(0.0),
            y: chip.y + CHIP_COLOR + 6.0,
            width: ancho,
            height: PALETA_PAD * 3.0 + COLOR_SLOT + TONO_ALTO,
        }
    }

    /// La fila de los diez acentos dentro del popover.
    fn rect_acentos(&self) -> iced_core::Rectangle {
        let p = self.rect_paleta();
        iced_core::Rectangle {
            x: p.x + PALETA_PAD,
            y: p.y + PALETA_PAD,
            width: TONO_ANCHO,
            height: COLOR_SLOT,
        }
    }

    /// La tira de tono libre, debajo de los diez.
    fn rect_tono(&self) -> iced_core::Rectangle {
        let p = self.rect_paleta();
        iced_core::Rectangle {
            x: p.x + PALETA_PAD,
            y: p.y + PALETA_PAD * 2.0 + COLOR_SLOT,
            width: TONO_ANCHO,
            height: TONO_ALTO,
        }
    }

    /// Qué color del selector cae en un punto, si es que hay alguno.
    ///
    /// El `Option` de dentro es el color elegido: los diez de la tabla y los
    /// 360 tonos de la tira. Volver a pulsar el que ya está puesto es lo que
    /// devuelve la carpeta al gris, y eso lo decide quien llama.
    fn color_en(&self, x: f32, y: f32) -> Option<ColorCarpeta> {
        if !self.paleta_abierta {
            return None;
        }
        let punto = iced_core::Point::new(x, y);
        let acentos = self.rect_acentos();
        if acentos.contains(punto) {
            let i = ((x - acentos.x) / COLOR_PASO) as usize;
            // El hueco entre ranuras no es de nadie.
            if x - acentos.x - i as f32 * COLOR_PASO > COLOR_SLOT {
                return None;
            }
            return tema::Acento::FIJOS
                .get(i)
                .copied()
                .map(ColorCarpeta::Acento);
        }
        let tono = self.rect_tono();
        if tono.contains(punto) {
            let grados = (x - tono.x) / tono.width * 360.0;
            return Some(ColorCarpeta::Tono(grados.clamp(0.0, 359.0)));
        }
        None
    }

    /// Cierra el selector. `true` si estaba abierto, para saber si hay que
    /// repintar —y para que Esc lo cierre antes que la carpeta—.
    fn cerrar_paleta(&mut self) -> bool {
        self.foco_paleta = None;
        std::mem::take(&mut self.paleta_abierta)
    }

    /// Los colores del selector en el orden en que los recorre el teclado: los
    /// diez de la tabla y luego seis tonos de la tira, uno cada 60°.
    ///
    /// Seis y no los 360 que se pueden pulsar con el ratón: recorrer una tira
    /// continua a golpe de flecha son doce pulsaciones para cruzar el rojo, y
    /// quien elige con el teclado quiere un color, no afinar el tono.
    fn colores_del_teclado() -> Vec<ColorCarpeta> {
        tema::Acento::FIJOS
            .into_iter()
            .map(ColorCarpeta::Acento)
            .chain(
                (0..TONOS_TECLADO)
                    .map(|i| ColorCarpeta::Tono(i as f32 * 360.0 / TONOS_TECLADO as f32)),
            )
            .collect()
    }

    /// Abre el selector con el foco puesto en el color que tenga la carpeta, o
    /// lo cierra si ya estaba. Es lo que hace el tabulador.
    fn alternar_paleta_con_teclado(&mut self, carpeta: usize) {
        if self.paleta_abierta {
            self.cerrar_paleta();
            return;
        }
        self.paleta_abierta = true;
        let puesto = self.carpetas[carpeta].color;
        self.foco_paleta = Some(
            Self::colores_del_teclado()
                .iter()
                .position(|c| Some(*c) == puesto)
                .unwrap_or(0),
        );
    }

    /// Mueve el foco del selector. Se queda en los extremos en vez de dar la
    /// vuelta: pasar del gris al azul de un salto se lee como un fallo.
    fn mover_foco_paleta(&mut self, paso: isize) {
        let colores = Self::colores_del_teclado();
        let actual = self.foco_paleta.unwrap_or(0) as isize;
        let nuevo = (actual + paso).clamp(0, colores.len() as isize - 1) as usize;
        self.foco_paleta = Some(nuevo);
    }

    /// Pone el color que tenga el foco. Como con el ratón, repetir el que ya
    /// está devuelve la carpeta al gris.
    fn aplicar_foco_paleta(&mut self) {
        let (Some(carpeta), Some(i)) = (self.dentro, self.foco_paleta) else {
            return;
        };
        let Some(color) = Self::colores_del_teclado().get(i).copied() else {
            return;
        };
        self.carpetas[carpeta].color =
            (self.carpetas[carpeta].color != Some(color)).then_some(color);
        self.cerrar_paleta();
        self.animar(Transicion::Fundido);
        self.guardar();
    }

    /// El rectángulo de la ✕ de la celda `i`, si el modo edición está puesto.
    fn cierre_de(&self, i: usize) -> Option<iced_core::Rectangle> {
        if !self.editando {
            return None;
        }
        let celda = self.rect_celda(i)?;
        // Arriba a la izquierda del icono, como en iOS: es la esquina que no
        // tapa ni el dibujo ni la etiqueta.
        let px = self.icono_px();
        Some(iced_core::Rectangle {
            x: celda.x + (celda.width - px) / 2.0 - CIERRE / 3.0,
            y: celda.y + (celda.height - ETIQUETA - px) / 2.0 - CIERRE / 3.0,
            width: CIERRE,
            height: CIERRE,
        })
    }

    /// Alterna el modo edición: con él salen las ✕. Lo abre el botón derecho.
    pub fn alternar_edicion(&mut self) -> bool {
        self.editando = !self.editando;
        self.animar(Transicion::Fundido);
        true
    }

    /// Quita una aplicación de la rejilla. **No la desinstala**: ver
    /// [`Launchpad::ocultas`].
    fn quitar(&mut self, app: usize) {
        let exec = self.apps[app].exec.clone();
        // Si estaba en una carpeta, sale de ella además de ocultarse: si no,
        // seguiría dentro y volvería a aparecer al deshacerla.
        for c in &mut self.carpetas {
            c.apps.retain(|a| a != &exec);
        }
        if !self.ocultas.contains(&exec) {
            self.ocultas.push(exec);
        }
        // La rejilla se recoloca entera al quitar algo: sin fundido, los iconos
        // de detrás dan un salto de una celda.
        self.animar(Transicion::Fundido);
        self.tras_cambiar_carpetas();
    }

    /// El rectángulo del nombre de la carpeta abierta, que es donde se pulsa
    /// para renombrarla.
    ///
    /// Es la mitad izquierda de la cabecera del panel: el nombre está ahí, y a
    /// la derecha vive el chip del color, que tiene su propio hit-test y manda
    /// sobre este porque se comprueba antes.
    fn rect_titulo(&self) -> iced_core::Rectangle {
        let panel = self.rect_panel();
        iced_core::Rectangle {
            x: panel.x + PANEL_PAD,
            y: panel.y + PANEL_PAD,
            width: panel.width - PANEL_PAD * 2.0 - CHIP_COLOR,
            height: PANEL_CABECERA,
        }
    }

    /// Empieza a renombrar la carpeta abierta.
    fn renombrar(&mut self) -> bool {
        let Some(c) = self.dentro else {
            return false;
        };
        self.renombrando = Some(self.carpetas[c].nombre.clone());
        true
    }

    /// Guarda el nombre escrito. Un nombre vacío deja el que había: una carpeta
    /// sin rótulo no se distingue de otra.
    fn confirmar_nombre(&mut self) {
        let (Some(nombre), Some(c)) = (self.renombrando.take(), self.dentro) else {
            return;
        };
        let nombre = nombre.trim();
        // Las claves reservadas de `launchpad.conf`: una carpeta llamada así
        // se leería al volver como la lista de ocultas o el orden.
        let reservado =
            [crate::config::CLAVE_OCULTAS, crate::config::CLAVE_ORDEN].contains(&nombre);
        if nombre.is_empty() || reservado || self.carpetas[c].nombre == nombre {
            return;
        }
        self.carpetas[c].nombre = nombre.to_string();
        self.tras_cambiar_carpetas();
    }

    /// Escribe el estado entero en `launchpad.conf`.
    fn guardar(&self) {
        let datos = crate::config::Launchpad {
            carpetas: self
                .carpetas
                .iter()
                .map(|c| {
                    (
                        c.nombre.clone(),
                        c.color.map(ColorCarpeta::guardado),
                        c.apps.clone(),
                    )
                })
                .collect(),
            ocultas: self.ocultas.clone(),
            orden: self.orden.clone(),
        };
        if let Err(err) = crate::config::guardar_launchpad(&datos) {
            tracing::warn!("no se pudo guardar el launchpad: {err}");
        }
    }

    /// Entra en una carpeta.
    fn abrir_carpeta(&mut self, carpeta: usize) {
        self.animar(Transicion::AbrirCarpeta);
        self.dentro = Some(carpeta);
        self.pagina = 0;
        self.seleccion = None;
        self.rehacer_visibles();
        self.preparar_pagina();
    }

    /// Resuelve los iconos de la página que se está viendo.
    ///
    /// Solo los treinta de la página, y una vez cada uno. Resolver los de todas
    /// las aplicaciones instaladas al abrir serían varios cientos de miles de
    /// `stat` —la búsqueda de freedesktop recorre directorios por temas por
    /// tamaños por categorías— y el launchpad tardaría en salir justo lo que no
    /// puede tardar.
    fn preparar_pagina(&mut self) {
        let desde = self.pagina * self.por_pagina();
        let hasta = (desde + self.por_pagina()).min(self.visibles.len());
        // Los de la página, y además los de dentro de las carpetas que se ven:
        // el icono de una carpeta **son** las miniaturas de lo que guarda, así
        // que sin ellos se dibujaría una caja vacía.
        let mut pendientes: Vec<usize> = Vec::new();
        for item in &self.visibles[desde..hasta] {
            match *item {
                Item::App(i) => pendientes.push(i),
                Item::Carpeta(c) => pendientes.extend(self.apps_de(c).into_iter().take(MINIATURAS)),
            }
        }
        for app in pendientes {
            self.iconos
                .entry(app)
                .or_insert_with(|| icono::cargar(&self.apps[app].icono));
        }
    }

    /// El **buffer** es solo el bloque de contenido, no la pantalla.
    ///
    /// Medido: rasterizar a pantalla completa cuesta 52 ms y señalar una celda
    /// otros 23, porque iced borra la máscara de recorte entera por cada capa y
    /// cada zona dañada — o sea que el coste va con el tamaño del buffer y no
    /// con lo que cambia, y el damage parcial no lo salva. El velo que ocupa la
    /// pantalla lo pinta el compositor con un color sólido, que en la GPU no
    /// cuesta nada, y aquí solo se rasteriza lo que tiene contenido.
    pub fn size(&self) -> (f32, f32) {
        (
            self.celda_ancho() * self.forma().0 as f32,
            BUSQUEDA + HUECO + self.celda_alto() * self.forma().1 as f32 + PUNTOS,
        )
    }

    /// El velo que va detrás, a pantalla completa. Lo pinta el compositor.
    ///
    /// **Se aparta del sistema de diseño a sabiendas.** §2.6 pide un scrim al
    /// 45-55 % con desenfoque de 6-12 px, y aquí va al 82 % sin desenfoque: el
    /// blur necesita un shader contra el `GlesRenderer` y todavía no existe.
    /// Sin él, un velo al 45 % deja el escritorio legible por debajo y las
    /// etiquetas del launchpad se pelean con lo que hay detrás. Cuando haya
    /// blur, esto baja a 0,45 y no antes: bajarlo ahora sería cumplir el token
    /// y romper lo que el token existe para conseguir.
    pub fn velo(&self) -> Color {
        // 0,45, que es lo que pide el sistema de diseño. Estuvo en 0,82 todo el
        // tiempo que el fondo salió nítido: sin desenfoque, un velo al 45 % no
        // separaba los iconos de lo que hubiera detrás. Ahora el compositor
        // desenfoca la pantalla entera bajo el launchpad y el velo vuelve a su
        // valor; con 0,82 encima, el desenfoque no se veía.
        let Some(tinte) = self.tinte_fondo else {
            return Color {
                a: 0.45,
                ..tema::bg()
            };
        };
        // Con el tono del fondo encima al 10 %. §2.6 fija el scrim en negro y
        // punto, y esto se aparta a sabiendas: sin el tinte, el launchpad se
        // lee como una lámina gris pegada sobre cualquier escritorio en vez de
        // como parte del que hay debajo.
        //
        // Los números salen de componer las dos capas en una, que es lo único
        // que el compositor sabe pintar aquí: un tinte al 10 % sobre el velo al
        // 45 % tapa 0,10 + 0,45 × 0,90 = 0,505, y de ese total el tinte pone
        // 0,10/0,505 ≈ 0,20.
        Color {
            a: 0.505,
            ..tema::mezclar(tema::bg(), tinte, 0.20)
        }
    }

    /// El realce de la celda señalada: su rectángulo lógico **relativo al
    /// buffer** y si va con marco de acento.
    ///
    /// Se devuelve en vez de dibujarlo porque mover el ratón por la rejilla
    /// repintaría el buffer entero. Como superficie aparte, señalar otra celda
    /// es mover un rectángulo de 200 px: gratis.
    pub fn realce(&self) -> Option<(iced_core::Rectangle, bool)> {
        let rect = self.rect_celda(self.seleccion?)?;
        // El marco de acento dice «soltar aquí hace algo». Arrastrando eso es
        // el destino sobre el que nacería la carpeta, y sin él el gesto no
        // tenía **ninguna** señal: el icono no sigue al cursor y el realce del
        // destino era el mismo gris que el de pasar el ratón por encima, así
        // que arrastrar y no arrastrar se veían igual y parecía que juntar dos
        // iconos no estuviera implementado.
        let soltando = self
            .arrastre
            .filter(|a| a.movido && Some(a.origen) != self.seleccion)
            .is_some_and(|_| self.destino_valido());
        Some((rect, soltando || !self.consulta.is_empty()))
    }

    /// ¿Lo señalado ahora mismo acepta que le suelten un icono encima?
    ///
    /// Una carpeta siempre; otra aplicación solo fuera de una carpeta, porque
    /// no hay carpetas dentro de carpetas.
    fn destino_valido(&self) -> bool {
        match self.seleccion.and_then(|i| self.visibles.get(i)) {
            Some(Item::Carpeta(_)) => true,
            Some(Item::App(_)) => self.dentro.is_none(),
            None => false,
        }
    }

    /// Tamaño lógico del realce: siempre el de una celda.
    pub fn realce_size(&self) -> (f32, f32) {
        (self.celda_ancho() - UNIDAD, self.celda_alto() - UNIDAD)
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// El área que ocupa la rejilla, y su esquina. Es el 72 % de la pantalla
    /// con un mínimo de 640×480, centrado.
    ///
    /// De alto, además, lo que deje libre el resto del launchpad —buscador, su
    /// hueco y los puntos— con [`AIRE`] arriba y abajo. El 72 % solo era de la
    /// rejilla y lo demás iba encima: a 1440×900 el bloque medía 828 en los
    /// 800 que deja el dock, y el buscador acababa a 2 px del borde.
    fn area(&self) -> (f32, f32) {
        let cabe = self.pantalla.1 - BUSQUEDA - HUECO - PUNTOS - AIRE * 2.0;
        (
            (self.pantalla.0 * FRACCION).max(640.0),
            (self.pantalla.1 * FRACCION).min(cabe).max(480.0),
        )
    }

    /// Columnas y filas de la rejilla principal.
    ///
    /// Un 6×5 fijo no se adapta: en 16:10 el alto es el eje que manda y cinco
    /// filas lo trocean hasta dejar iconos de 63 px con medio ancho vacío. Así
    /// que se quitan filas cuando no caben a [`FILA_MINIMA`], y las columnas
    /// salen de cuántas celdas un 10 % más anchas que altas caben a lo ancho:
    /// 7×4 a 1440×900 y a 1280×800, 7×5 a 1920×1080, 4×5 en vertical.
    fn forma(&self) -> (usize, usize) {
        let (ancho, alto) = self.area();
        let filas = if alto / 5.0 >= FILA_MINIMA { 5 } else { 4 };
        let columnas = (ancho / (alto / filas as f32 * 1.1)).round() as usize;
        (columnas.clamp(4, COLUMNAS_MAX), filas)
    }

    /// El paso **vertical**: lo que mide una fila.
    ///
    /// Sale del alto, con el ancho de tope por si la pantalla es vertical. De
    /// aquí salen también el tamaño del icono y el aire de la celda, y por eso
    /// no es simplemente `alto / filas`: es el eje que manda.
    fn paso(&self) -> f32 {
        let (ancho, alto) = self.area();
        let (columnas, filas) = self.forma();
        (alto / filas as f32).min(ancho / columnas as f32).max(80.0)
    }

    /// El paso **horizontal**: lo que mide una columna.
    ///
    /// Más ancho que alto, hasta un tercio más. Los dos extremos ya se han
    /// visto en pantalla y los dos están mal: con el 0,72 aplicado a cada eje
    /// por su cuenta —lo que había— la columna sale 1,33 veces la fila **y** el
    /// icono se encoge al mínimo de los dos ejes, o sea 57 px con 116 de aire a
    /// los lados, y las filas se leen como bandas; con la celda cuadrada la
    /// rejilla se queda en la mitad central de una pantalla ancha y sobra medio
    /// escritorio a los lados.
    ///
    /// Aquí el icono lo fija el paso vertical —así no se encoge— y la columna
    /// se estira hasta donde da el 0,72 del ancho, con el tope de 1,35 para que
    /// el hueco horizontal no llegue a doblar al vertical, que es cuando
    /// vuelven las bandas.
    fn paso_x(&self) -> f32 {
        let paso = self.paso();
        (self.area().0 / self.forma().0 as f32).clamp(paso, paso * 1.35)
    }

    fn celda_ancho(&self) -> f32 {
        self.paso_x()
    }

    fn celda_alto(&self) -> f32 {
        self.paso()
    }

    /// El icono: **el 58 % del paso vertical**, no todo lo que quepa.
    ///
    /// Con «todo lo que quepa» —el paso menos las tres unidades de la etiqueta y
    /// su aire— el icono se come la celda: a 2000×1250 el paso es 180, el icono
    /// sale 126 y con el rótulo de dos líneas quedan **6 px** por arriba y por
    /// abajo. Visto en pantalla: las filas se tocan y la rejilla se lee
    /// apelmazada. Con el 58 % el icono baja a 104 y el aire sube a 16, que es
    /// la proporción de los mockups y la que deja respirar a las etiquetas de
    /// dos líneas —«KDE Partition Manager» y compañía—.
    ///
    /// El mínimo de las tres unidades sigue estando de tope: en una pantalla
    /// muy baja el 58 % no cabría con la etiqueta.
    fn icono_px(&self) -> f32 {
        (self.paso() * 0.58)
            .min(self.paso() - UNIDAD * 3.0)
            .max(48.0)
    }

    /// Columnas y filas de la rejilla que se está viendo. Dentro de una
    /// carpeta son las del panel, que tiene las suyas.
    fn columnas(&self) -> usize {
        match self.dentro {
            // Las que ocupe lo que hay dentro, hasta cuatro: con cuatro
            // siempre, una carpeta de dos aplicaciones es un panel con media
            // fila vacía y las dos pegadas a la izquierda.
            Some(_) => self.visibles.len().clamp(1, COLUMNAS_CARPETA),
            None => self.forma().0,
        }
    }

    fn filas(&self) -> usize {
        if self.dentro.is_some() {
            FILAS_CARPETA
        } else {
            self.forma().1
        }
    }

    fn por_pagina(&self) -> usize {
        self.columnas() * self.filas()
    }

    /// La esquina de la rejilla que se está viendo, en el buffer.
    ///
    /// Fuera de una carpeta empieza en el borde. Dentro, en el panel — que va
    /// centrado— pero **a la misma altura**: si la rejilla de dentro no
    /// arrancara donde la de fuera, los iconos saltarían al entrar y al salir.
    fn origen_rejilla(&self) -> (f32, f32) {
        match self.dentro {
            Some(_) => {
                let panel = self.rect_panel();
                (panel.x + PANEL_PAD, panel.y + PANEL_PAD + PANEL_CABECERA)
            }
            None => (self.margen_x(), self.margen_y() + BUSQUEDA + HUECO),
        }
    }

    /// Lo que se aparta una fila para quedar centrada, dentro de una carpeta.
    ///
    /// Solo la afecta a la última, que es la única que puede ir a medias. Fuera
    /// de la carpeta es siempre cero: la rejilla grande se lee como una tabla y
    /// centrar su última fila descolocaría las columnas.
    fn hueco_fila(&self, fila: usize) -> f32 {
        if self.dentro.is_none() {
            return 0.0;
        }
        let columnas = self.columnas();
        let primero = self.pagina * self.por_pagina() + fila * columnas;
        let quedan = self.visibles.len().saturating_sub(primero).min(columnas);
        if quedan == 0 {
            return 0.0;
        }
        (columnas - quedan) as f32 * self.celda_ancho() / 2.0
    }

    /// Cuántas filas de alto tiene el panel de la carpeta abierta.
    ///
    /// Las que ocupe lo que hay dentro, hasta el máximo de la página. Con las
    /// tres siempre, una carpeta de dos aplicaciones es un panel con dos
    /// tercios de vacío. Se cuenta sobre el **total** y no sobre la página que
    /// se ve, para que pasar de página no cambie el tamaño del panel.
    fn filas_panel(&self) -> usize {
        self.visibles
            .len()
            .div_ceil(self.columnas().max(1))
            .clamp(1, FILAS_CARPETA)
    }

    /// El panel de la carpeta abierta: la tarjeta que envuelve la cabecera y su
    /// rejilla. Es un diálogo, así que lleva el radio y la sombra de uno.
    fn rect_panel(&self) -> iced_core::Rectangle {
        let paso = self.paso();
        let ancho = self.columnas() as f32 * self.celda_ancho() + PANEL_PAD * 2.0;
        // Con más de una página, dentro caben también sus puntos.
        let puntos = if self.paginas() > 1 { PUNTOS } else { 0.0 };
        let alto = PANEL_CABECERA + self.filas_panel() as f32 * paso + PANEL_PAD * 2.0 + puntos;
        // Centrado en los dos ejes. Estuvo colgado de la altura de la rejilla
        // —para que los iconos de dentro empezaran donde los de fuera— y el
        // resultado era un panel pequeño pegado al borde de arriba con media
        // pantalla vacía debajo: una carpeta abierta es un diálogo, y un
        // diálogo va en el centro.
        iced_core::Rectangle {
            x: (self.size().0 - ancho) / 2.0,
            y: ((self.size().1 - alto) / 2.0).max(0.0),
            width: ancho,
            height: alto,
        }
    }

    /// La rejilla empieza en el borde del buffer: el centrado en la pantalla lo
    /// hace el anclaje, no un margen dibujado.
    fn margen_x(&self) -> f32 {
        0.0
    }

    fn margen_y(&self) -> f32 {
        0.0
    }

    fn paginas(&self) -> usize {
        self.visibles.len().div_ceil(self.por_pagina()).max(1)
    }

    /// Rehace la lista visible tras cambiar la búsqueda.
    fn filtrar(&mut self) {
        let mut con_puntos: Vec<(u32, usize)> = self
            .apps
            .iter()
            .enumerate()
            // Lo quitado no sale tampoco buscando: si saliera, la ✕ no habría
            // servido de nada.
            .filter(|(_, app)| !self.ocultas.contains(&app.exec))
            .filter_map(|(i, app)| apps::puntuar(app, &self.consulta).map(|p| (p, i)))
            .collect();
        // Entre los aciertos buenos (nombre, prefijo, palabra o iniciales)
        // manda el uso: con «vi», VirtualBox antes que Visor si es la que se
        // abre. Lo que solo coincide a mitad de palabra o en difuso sigue
        // detrás aunque se use mucho, o una letra suelta lo traería todo.
        // A igualdad, alfabético: `apps` ya viene ordenado y la ordenación es
        // estable.
        con_puntos.sort_by_key(|&(p, i)| {
            let usos = self.usos.get(&self.apps[i].exec).copied().unwrap_or(0);
            (p > 3, std::cmp::Reverse(usos), p)
        });
        self.visibles = con_puntos.into_iter().map(|(_, i)| Item::App(i)).collect();
        // Sin nada escrito se vuelve a la rejilla de siempre —carpetas
        // incluidas—, que es lo que `rehacer_visibles` sabe montar.
        if self.consulta.is_empty() {
            self.rehacer_visibles();
        }
        self.pagina = 0;
        self.preparar_pagina();
        // Con búsqueda activa se preselecciona el primero: escribir y pulsar
        // Intro tiene que lanzar lo que se está viendo arriba a la izquierda.
        self.seleccion = (!self.consulta.is_empty() && !self.visibles.is_empty()).then_some(0);
    }

    /// Qué celda de la página cae en un punto lógico.
    fn celda_en(&self, x: f32, y: f32) -> Option<usize> {
        self.posicion_en(x, y).filter(|&i| i < self.visibles.len())
    }

    /// Como [`Launchpad::celda_en`], pero también las celdas vacías del final
    /// de la última página: soltar ahí manda el icono al final.
    fn posicion_en(&self, x: f32, y: f32) -> Option<usize> {
        let (x0, y0) = self.origen_rejilla();
        if y < y0 {
            return None;
        }
        // La fila primero: dentro de una carpeta, cuánto se aparta la fila
        // depende de cuántos iconos le queden, así que sin saber la fila no se
        // puede saber la columna.
        let fila = ((y - y0) / self.celda_alto()) as usize;
        let x0 = x0 + self.hueco_fila(fila);
        if x < x0 {
            return None;
        }
        let col = ((x - x0) / self.celda_ancho()) as usize;
        if col >= self.columnas() || fila >= self.filas() {
            return None;
        }
        let en_pagina = fila * self.columnas() + col;
        Some(self.pagina * self.por_pagina() + en_pagina)
    }

    /// Dónde se colocaría lo arrastrado si se suelta en `(x, y)`: la posición
    /// en `visibles` delante de la que se insertaría. `None` si ahí no se
    /// recoloca —sobre el icono crea carpeta, y fuera de la rejilla nada—.
    ///
    /// Solo cuenta lo horizontal: el icono ocupa el centro de la celda y lo
    /// que queda a los lados es el hueco entre dos, que es donde se apunta
    /// para meter algo en medio.
    fn hueco_en(&self, x: f32, y: f32) -> Option<usize> {
        if self.dentro.is_some() || !self.consulta.is_empty() {
            return None;
        }
        let i = self.posicion_en(x, y)?;
        if i >= self.visibles.len() {
            return Some(self.visibles.len());
        }
        let medio = self.rect_celda(i)?.center_x();
        if (x - medio).abs() <= self.icono_px() / 2.0 {
            return None;
        }
        Some(if x < medio { i } else { i + 1 })
    }

    /// Recoloca la aplicación de `origen` delante de la posición `hasta`.
    ///
    /// Se guarda el orden **entero** de las sueltas y no solo la que se ha
    /// movido: «esta va en la 31» dejaría de ser cierto en cuanto se instalase
    /// algo que cae antes alfabéticamente.
    fn mover_a(&mut self, origen: usize, hasta: usize) -> bool {
        let Some(&Item::App(app)) = self.visibles.get(origen) else {
            // Las carpetas van siempre primero: no tienen a dónde moverse.
            return true;
        };
        let carpetas = self.carpetas.len();
        let mut sueltas: Vec<usize> = self
            .visibles
            .iter()
            .filter_map(|v| match *v {
                Item::App(a) => Some(a),
                Item::Carpeta(_) => None,
            })
            .collect();
        // Quitarla primero corre una posición todo lo que iba detrás de ella.
        let mut hasta = hasta.saturating_sub(carpetas);
        let desde = origen - carpetas;
        if desde < hasta {
            hasta -= 1;
        }
        sueltas.remove(desde);
        sueltas.insert(hasta.min(sueltas.len()), app);
        self.orden = sueltas.iter().map(|&a| self.apps[a].exec.clone()).collect();
        self.guardar();
        self.rehacer_visibles();
        let nueva = carpetas + hasta.min(sueltas.len() - 1);
        self.pagina = nueva / self.por_pagina();
        self.seleccion = Some(nueva);
        self.preparar_pagina();
        true
    }

    /// La página que se está viendo. Para que el shell sepa si el arrastre la
    /// ha cambiado y hay que repintar la rejilla.
    pub fn pagina(&self) -> usize {
        self.pagina
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if let (Some(arrastre), Some((x, y))) = (self.arrastre.as_mut(), punto) {
            arrastre.actual = (x, y);
            let (dx, dy) = (x - arrastre.inicio.0, y - arrastre.inicio.1);
            if !arrastre.movido && dx.hypot(dy) > UMBRAL_ARRASTRE {
                arrastre.movido = true;
            }
            let movido = arrastre.movido;
            if movido && self.pasar_por_borde(x) {
                return true;
            }
            // Mientras se arrastra, lo señalado es **el destino**: es lo que
            // dice sobre qué se va a soltar. El realce va en su propia
            // superficie, así que moverlo no repinta la rejilla.
            let destino = (!self.sobre_el_dock(x, y))
                .then(|| self.celda_en(x, y))
                .flatten();
            if destino.is_some() && destino != self.seleccion {
                self.seleccion = destino;
                return true;
            }
            // Con el icono en el aire se contesta que sí aunque no cambie el
            // destino: hay que recolocarlo en cada movimiento. No repinta la
            // rejilla —`emergente_needs_paint` sigue diciendo que no—, solo
            // mueve su superficie, que es lo que hace que seguir al cursor sea
            // gratis.
            return movido;
        }
        let seleccion = punto.and_then(|(x, y)| self.celda_en(x, y));
        // Sin puntero encima se conserva la selección del teclado: si el ratón
        // saliendo del área la borrase, escribir y mover el ratón sin querer
        // dejaría a Intro sin destino.
        if seleccion.is_none() || self.seleccion == seleccion {
            return false;
        }
        self.seleccion = seleccion;
        true
    }

    /// Con un icono en el aire que sale de la rejilla por la izquierda o por la
    /// derecha, pasa de página, que es la única forma de llevarlo a otra: con
    /// el botón pulsado no hay gesto de touchpad. Si se sigue moviendo fuera,
    /// pasa otra cada 700 ms, lo bastante despacio para volver a la que se
    /// quiere.
    ///
    /// Fuera y no «cerca del borde»: agarrado, el punto sigue llegando aunque
    /// se salga del buffer —la rejilla es el 72 % central—, y una franja por
    /// dentro se comería el hueco izquierdo de la primera columna, que es
    /// donde se suelta para poner algo el primero.
    fn pasar_por_borde(&mut self, x: f32) -> bool {
        const CADA: std::time::Duration = std::time::Duration::from_millis(700);
        if self.dentro.is_some() || !self.consulta.is_empty() {
            return false;
        }
        let hacia: isize = if x < 0.0 {
            -1
        } else if x > self.size().0 {
            1
        } else {
            self.paso_por_borde = None;
            return false;
        };
        if self.paso_por_borde.is_some_and(|t| t.elapsed() < CADA) {
            return false;
        }
        let destino = self.pagina as isize + hacia;
        if destino < 0 || destino >= self.paginas() as isize {
            return false;
        }
        self.pagina = destino as usize;
        self.seleccion = None;
        self.preparar_pagina();
        self.paso_por_borde = Some(std::time::Instant::now());
        true
    }

    /// Los nombres de los primeros elementos visibles. Para el autotest: es lo
    /// que dice si una carpeta ha nacido y dónde.
    pub fn primeros(&self, n: usize) -> Vec<String> {
        self.visibles
            .iter()
            .take(n)
            .map(|i| match *i {
                Item::App(a) => self.apps[a].nombre.clone(),
                Item::Carpeta(c) => self.carpetas[c].nombre.clone(),
            })
            .collect()
    }

    /// El centro de una celda de la página actual, para el autotest: sin esto
    /// no hay forma de decir «pulsa el primer icono» desde fuera.
    pub fn centro_de_celda(&self, i: usize) -> Option<(f32, f32)> {
        self.rect_celda(i).map(|r| (r.center_x(), r.center_y()))
    }

    /// El centro del chip del selector de color, y el de un tono de la tira.
    /// Para el autotest: sin esto no hay forma de pulsar el selector desde
    /// fuera, y sus dos rectángulos dependen del tamaño de la pantalla.
    pub fn centro_selector(&self) -> (f32, f32) {
        let r = self.rect_chip_color();
        (r.center_x(), r.center_y())
    }

    pub fn centro_tono(&self, grados: f32) -> (f32, f32) {
        let r = self.rect_tono();
        (
            r.x + r.width * grados.clamp(0.0, 359.0) / 360.0,
            r.center_y(),
        )
    }

    /// El rectángulo lógico de una celda de la página actual.
    fn rect_celda(&self, indice: usize) -> Option<iced_core::Rectangle> {
        let en_pagina = indice.checked_sub(self.pagina * self.por_pagina())?;
        if en_pagina >= self.por_pagina() {
            return None;
        }
        let (fila, col) = (en_pagina / self.columnas(), en_pagina % self.columnas());
        let (x0, y0) = self.origen_rejilla();
        Some(iced_core::Rectangle {
            x: x0 + self.hueco_fila(fila) + col as f32 * self.celda_ancho(),
            y: y0 + fila as f32 * self.celda_alto(),
            width: self.celda_ancho(),
            height: self.celda_alto(),
        })
    }

    /// Pulsar **no lanza nada**: agarra. Lo que hace el icono se decide al
    /// soltar, porque hasta entonces no se sabe si esto era un clic o el
    /// principio de un arrastre.
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        // La ✕ manda sobre la celda: está encima de su icono, y si ganara el
        // icono no habría forma de pulsarla.
        if self.editando {
            let cerrar = (0..self.visibles.len())
                .find(|i| {
                    self.cierre_de(*i)
                        .is_some_and(|r| r.contains(iced_core::Point::new(x, y)))
                })
                .and_then(|i| match self.visibles.get(i) {
                    Some(Item::App(a)) => Some(*a),
                    // Una carpeta no se quita con la ✕: se deshace sacando lo
                    // que tiene dentro, y así no se pierde de vista nada sin
                    // querer.
                    _ => None,
                });
            if let Some(app) = cerrar {
                self.quitar(app);
                return None;
            }
        }
        // La ✕ del buscador limpia lo escrito. Va antes que la rejilla porque
        // está encima de ella —el campo, no—.
        if self
            .rect_limpiar()
            .is_some_and(|r| r.contains(iced_core::Point::new(x, y)))
        {
            self.consulta.clear();
            self.filtrar();
            return None;
        }
        // El selector de color de la carpeta abierta, en tres pasos: el chip
        // lo despliega, un color de dentro lo elige, y pulsar fuera lo cierra
        // sin tocar nada —que es lo que hace cualquier popover—.
        if let Some(c) = self.dentro {
            if self.rect_chip_color().contains(iced_core::Point::new(x, y)) {
                self.paleta_abierta = !self.paleta_abierta;
                return None;
            }
            if let Some(color) = self.color_en(x, y) {
                // Volver a pulsar el color que ya tiene lo quita: es la forma
                // de devolverla al gris sin un botón de «ninguno».
                self.carpetas[c].color = (self.carpetas[c].color != Some(color)).then_some(color);
                self.paleta_abierta = false;
                self.animar(Transicion::Fundido);
                self.guardar();
                return None;
            }
            if self.cerrar_paleta() {
                return None;
            }
        }
        // Dentro de una carpeta, pulsar su nombre lo pone a editar. Pulsar en
        // cualquier otro sitio confirma lo escrito, como cualquier campo.
        if self.dentro.is_some() {
            let en_titulo = self.rect_titulo().contains(iced_core::Point::new(x, y));
            if self.renombrando.is_some() && !en_titulo {
                self.confirmar_nombre();
            } else if en_titulo {
                self.renombrar();
                return None;
            }
        }
        // Sobre el dock no se agarra nada de la rejilla: ese clic es del dock,
        // y quien lo enruta es el compositor, que es el único que tiene las dos
        // superficies. Sin esto, pulsar un icono del dock con el launchpad
        // abierto empezaba un arrastre de la celda que hubiera detrás.
        if self.sobre_el_dock(x, y) {
            return None;
        }
        let i = self.celda_en(x, y)?;
        self.seleccion = Some(i);
        self.arrastre = Some(Arrastre {
            origen: i,
            inicio: (x, y),
            actual: (x, y),
            movido: false,
        });
        None
    }

    /// ¿Hay un icono agarrado? Mientras lo haya, el puntero le sigue llegando
    /// aunque se salga de la rejilla: soltar fuera es lo que saca una
    /// aplicación de su carpeta.
    pub fn agarrado(&self) -> bool {
        self.arrastre.is_some()
    }

    /// Se ha soltado el botón. Devuelve si hay que repintar y qué hacer.
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        let Some(arrastre) = self.arrastre.take() else {
            return (false, None);
        };
        if !arrastre.movido {
            // No se movió: era un clic de toda la vida.
            return (true, self.activar(arrastre.origen));
        }
        let (x, y) = arrastre.actual;
        // Soltar sobre el dock ancla, y se mira **antes** que la rejilla: el
        // dock queda fuera de ella, y sin esta rama lo que pasaría es que el
        // icono se saliera de su carpeta, que es lo que significa soltar fuera.
        if self.sobre_el_dock(x, y) {
            return (true, self.anclar_en_el_dock(arrastre.origen));
        }
        self.paso_por_borde = None;
        if let Some(hasta) = self.hueco_en(x, y) {
            return (self.mover_a(arrastre.origen, hasta), None);
        }
        let destino = self.celda_en(x, y);
        (self.soltar_en(arrastre.origen, destino), None)
    }

    /// Qué pasa al soltar lo de `origen` sobre `destino`.
    ///
    /// Las tres cosas que se pueden hacer con una carpeta, y ninguna más:
    /// crearla juntando dos aplicaciones, meter otra dentro, y sacar una
    /// soltándola fuera de la rejilla. Recolocar no pasa por aquí: lo decide
    /// [`Launchpad::hueco_en`] antes, según se suelte en el icono o a su lado.
    fn soltar_en(&mut self, origen: usize, destino: Option<usize>) -> bool {
        let Some(&item_origen) = self.visibles.get(origen) else {
            return false;
        };
        let Item::App(app) = item_origen else {
            // Arrastrar una carpeta entera no hace nada: no hay carpetas dentro
            // de carpetas, y moverla de sitio no tendría dónde guardarse.
            return true;
        };
        let exec = self.apps[app].exec.clone();

        // Fuera de la rejilla, estando dentro de una carpeta: se saca de ella.
        let Some(destino) = destino else {
            if let Some(c) = self.dentro {
                self.sacar_de_carpeta(c, &exec);
            }
            return true;
        };
        if destino == origen {
            return true;
        }
        match self.visibles.get(destino).copied() {
            // Sobre otra aplicación: nace una carpeta con las dos.
            Some(Item::App(otra)) if self.dentro.is_none() => {
                let con = self.apps[otra].exec.clone();
                self.crear_carpeta(&con, &exec);
            }
            // Sobre una carpeta: se mete dentro.
            Some(Item::Carpeta(c)) => self.meter_en_carpeta(c, &exec),
            _ => {}
        }
        true
    }

    /// Crea una carpeta con dos aplicaciones. La primera da el nombre inicial.
    fn crear_carpeta(&mut self, primera: &str, segunda: &str) {
        // «Carpeta» a secas, como macOS cuando no puede deducir una categoría:
        // adivinarla de las categorías del `.desktop` acierta poco —media
        // distribución declara `Utility`— y un nombre equivocado es peor que
        // uno neutro que se cambia en dos segundos.
        self.carpetas.push(Carpeta {
            nombre: "Carpeta".into(),
            color: None,
            apps: vec![primera.to_string(), segunda.to_string()],
        });
        self.tras_cambiar_carpetas();
        // Y se va a verla. Las carpetas se ordenan las primeras, así que crear
        // una desde la tercera página reordena la rejilla entera y deja al
        // usuario mirando un sitio donde no ha cambiado nada: el gesto parecía
        // no haber hecho nada cuando en realidad había funcionado.
        if let Some(i) = self
            .visibles
            .iter()
            .position(|v| matches!(v, Item::Carpeta(c) if self.carpetas[*c].apps.iter().any(|a| a == primera)))
        {
            self.pagina = i / self.por_pagina();
            self.seleccion = Some(i);
            self.preparar_pagina();
        }
    }

    fn meter_en_carpeta(&mut self, carpeta: usize, exec: &str) {
        // Antes se saca de donde estuviera: una aplicación está en una carpeta
        // o en ninguna, nunca en dos.
        for c in &mut self.carpetas {
            c.apps.retain(|a| a != exec);
        }
        if let Some(c) = self.carpetas.get_mut(carpeta) {
            c.apps.push(exec.to_string());
        }
        self.tras_cambiar_carpetas();
    }

    fn sacar_de_carpeta(&mut self, carpeta: usize, exec: &str) {
        if let Some(c) = self.carpetas.get_mut(carpeta) {
            c.apps.retain(|a| a != exec);
        }
        self.tras_cambiar_carpetas();
    }

    /// Limpia, guarda y rehace la vista. Se llama tras **cualquier** cambio en
    /// las carpetas, que es lo que garantiza que el disco y lo que se ve digan
    /// lo mismo.
    fn tras_cambiar_carpetas(&mut self) {
        // Una carpeta con una sola aplicación no es una carpeta: se deshace
        // sola, como en macOS al sacar la penúltima.
        let dentro_de = self.dentro.and_then(|c| self.carpetas.get(c)).cloned();
        self.carpetas.retain(|c| c.apps.len() > 1);
        // Si la que estaba abierta ha desaparecido, se sale; si sigue, se
        // recoloca por nombre, que es lo único estable tras el `retain`.
        self.dentro = dentro_de.and_then(|antigua| {
            self.carpetas
                .iter()
                .position(|c| c.nombre == antigua.nombre && c.apps.len() > 1)
        });
        self.guardar();
        self.seleccion = None;
        self.rehacer_visibles();
        self.pagina = self.pagina.min(self.paginas() - 1);
        self.preparar_pagina();
    }

    /// Lo que hace pulsar la celda `i`: lanzar la aplicación, o entrar en la
    /// carpeta —que no es una acción para el compositor, sino un cambio de
    /// vista aquí dentro—.
    /// Lo que hay en `i`, listo para fijarlo al dock.
    ///
    /// Una carpeta no se ancla: no es un programa, y el dock no sabe abrir algo
    /// que solo existe dentro del launchpad.
    ///
    /// El `app_id` es el `exec`. Es lo mismo que hace la configuración cuando
    /// la línea del dock no lo trae —`firefox` a secas ya acierta—, y aquí no
    /// hay nada mejor: el `.desktop` no lleva el `app_id` de Wayland, lo lleva
    /// la ventana cuando aparece.
    fn anclar_en_el_dock(&self, i: usize) -> Option<Accion> {
        let Item::App(a) = *self.visibles.get(i)? else {
            return None;
        };
        let app = self.apps.get(a)?;
        Some(Accion::Anclar {
            app_id: app.exec.clone(),
            exec: app.exec.clone(),
            icono: app.icono.clone(),
            fijar: Some(true),
        })
    }

    fn activar(&mut self, i: usize) -> Option<Accion> {
        match *self.visibles.get(i)? {
            Item::App(a) => Some(Accion::Lanzar(self.apps[a].exec.clone())),
            Item::Carpeta(c) => {
                self.abrir_carpeta(c);
                None
            }
        }
    }

    /// El cambio de página no anima dentro del buffer.
    ///
    /// Rasterizar esta rejilla completa cuesta decenas de milisegundos. Hacerlo
    /// en cada frame bloqueaba el hilo del compositor durante toda la
    /// transición y se notaba incluso en el cursor, especialmente a 120 Hz.
    /// Una animación correcta necesitará dos superficies ya rasterizadas y
    /// moverlas en la GPU; hasta entonces el salto inmediato es mucho más
    /// fluido que una falsa animación hecha por CPU.
    pub fn animando(&self) -> bool {
        false
    }

    /// Desplazamiento del touchpad o de la rueda, en píxeles lógicos.
    ///
    /// Devuelve `true` si ha cambiado de página. **Una página por gesto**: tras
    /// el salto se traga todo hasta que pasan 180 ms sin eventos, que es
    /// levantar los dedos. Antes la espera contaba desde el salto, y un gesto
    /// largo —dura más de 180 ms— pasaba dos o tres páginas. libinput no tiene
    /// inercia (esa la simulan los clientes), así que mientras llegan eventos
    /// seguidos es el mismo gesto. Con la rueda cada muesca va separada y pasa
    /// su página, como antes.
    pub fn desplazar(&mut self, dx: f32, dy: f32) -> bool {
        const UMBRAL: f32 = 20.0;
        const ESPERA: std::time::Duration = std::time::Duration::from_millis(180);

        // El eje vertical también pasa de página: en un touchpad el gesto
        // natural sobre una rejilla es el horizontal, pero con rueda de ratón
        // solo hay vertical y quedarse quieto sería raro.
        let delta = if dx.abs() >= dy.abs() { dx } else { dy };
        let ahora = std::time::Instant::now();
        if let Some(anterior) = self.gesto_gastado {
            if anterior.elapsed() < ESPERA {
                self.gesto_gastado = Some(ahora);
                self.acumulado = 0.0;
                return false;
            }
            self.gesto_gastado = None;
        }
        self.acumulado += delta;
        if self.acumulado.abs() < UMBRAL {
            return false;
        }
        let hacia = if self.acumulado > 0.0 { 1 } else { -1 };
        self.acumulado = 0.0;
        let destino = self.pagina as isize + hacia;
        if destino < 0 || destino >= self.paginas() as isize {
            return false;
        }
        self.pagina = destino as usize;
        self.preparar_pagina();
        self.gesto_gastado = Some(ahora);
        true
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        // Renombrando, el teclado es del campo: escribir no busca y Esc no
        // cierra nada, solo deja el nombre como estaba.
        if let Some(nombre) = self.renombrando.as_mut() {
            return match tecla {
                T::Caracter(c) => {
                    nombre.push(c);
                    Tecla::Consumida
                }
                T::Retroceso => {
                    nombre.pop();
                    Tecla::Consumida
                }
                T::BorrarTodo => {
                    nombre.clear();
                    Tecla::Consumida
                }
                T::Intro => {
                    self.confirmar_nombre();
                    Tecla::Consumida
                }
                T::Escape => {
                    self.renombrando = None;
                    Tecla::Consumida
                }
                _ => Tecla::Consumida,
            };
        }
        match tecla {
            // Esc limpia la búsqueda antes de cerrar, como el plasmoide: si has
            // escrito algo, lo primero que quieres deshacer es eso.
            T::Escape if !self.consulta.is_empty() => {
                self.consulta.clear();
                self.filtrar();
                Tecla::Consumida
            }
            // Esc deshace lo último que hayas hecho, en orden: primero el
            // selector de color, luego el modo edición, luego la carpeta
            // abierta, y solo entonces cierra.
            T::Escape if self.cerrar_paleta() => Tecla::Consumida,
            T::Escape if self.editando => {
                self.editando = false;
                Tecla::Consumida
            }
            T::Escape if self.salir_de_carpeta() => Tecla::Consumida,
            T::Escape => Tecla::Cerrar,
            T::Caracter(c) => {
                // Escribir busca en todas las aplicaciones, también estando
                // dentro de una carpeta: quien escribe «kate» quiere Kate, no
                // que le recuerden en qué carpeta lo guardó.
                self.dentro = None;
                self.consulta.push(c);
                self.filtrar();
                Tecla::Consumida
            }
            T::Retroceso => {
                self.consulta.pop();
                self.filtrar();
                Tecla::Consumida
            }
            T::BorrarTodo => {
                self.consulta.clear();
                self.filtrar();
                Tecla::Consumida
            }
            // Con el selector desplegado, las flechas y el Intro son suyos: es
            // lo que hay delante, y mover la rejilla por detrás mientras se
            // elige un color no lo espera nadie.
            T::Izquierda if self.paleta_abierta => {
                self.mover_foco_paleta(-1);
                Tecla::Consumida
            }
            T::Derecha if self.paleta_abierta => {
                self.mover_foco_paleta(1);
                Tecla::Consumida
            }
            T::Intro if self.paleta_abierta => {
                self.aplicar_foco_paleta();
                Tecla::Consumida
            }
            T::Izquierda => self.mover(-1),
            T::Derecha => self.mover(1),
            T::Arriba => self.mover(-(self.columnas() as isize)),
            T::Abajo => self.mover(self.columnas() as isize),
            // Inicio y Fin van al primero y al último de todo, no de la página:
            // lo que se busca con Fin es lo último que hay instalado.
            T::Inicio => self.mover(-(self.visibles.len() as isize)),
            T::Fin => self.mover(self.visibles.len() as isize),
            T::PaginaArriba => self.pasar_pagina(-1),
            T::PaginaAbajo => self.pasar_pagina(1),
            T::SeleccionarTodo => Tecla::Ignorada,
            // El tabulador despliega el selector de color de la carpeta
            // abierta. Es la única tecla libre ahí: las letras buscan, las
            // flechas recorren la rejilla y Esc va deshaciendo.
            T::Tabulador => {
                if let Some(c) = self.dentro {
                    self.alternar_paleta_con_teclado(c);
                }
                Tecla::Consumida
            }
            T::Intro => match self.seleccionado() {
                Some(accion) => Tecla::Hacer(accion),
                None => Tecla::Consumida,
            },
        }
    }

    /// La acción de lo que esté seleccionado, para Intro.
    pub fn seleccionado(&mut self) -> Option<Accion> {
        let i = self.seleccion?;
        self.activar(i)
    }

    /// Cambia de página con el teclado y se lleva la selección con ella: sin
    /// eso, Intro después de AvPág lanzaría algo de la página anterior, que ya
    /// no se está viendo.
    fn pasar_pagina(&mut self, hacia: isize) -> Tecla {
        let destino = self.pagina as isize + hacia;
        if destino < 0 || destino >= self.paginas() as isize {
            return Tecla::Consumida;
        }
        self.pagina = destino as usize;
        if self.seleccion.is_some() {
            let primera = self.pagina * self.por_pagina();
            self.seleccion = Some(primera.min(self.visibles.len().saturating_sub(1)));
        }
        self.preparar_pagina();
        Tecla::Consumida
    }

    fn mover(&mut self, paso: isize) -> Tecla {
        if self.visibles.is_empty() {
            return Tecla::Consumida;
        }
        let actual = self.seleccion.unwrap_or(0) as isize;
        let nuevo = (actual + paso).clamp(0, self.visibles.len() as isize - 1) as usize;
        self.seleccion = Some(nuevo);
        // Salirse de la página la pasa: la selección manda sobre la página, no
        // al revés.
        self.pagina = nuevo / self.por_pagina();
        self.preparar_pagina();
        Tecla::Consumida
    }

    pub fn view(&self) -> PanelElement<'_> {
        // Dentro de una carpeta, arriba va su nombre y no el buscador: es lo
        // que dice dónde estás, y buscar desde dentro sale igualmente porque
        // escribir te saca a la rejilla completa.
        if let Some(c) = self.dentro {
            return self.view_carpeta(c);
        }
        let escribiendo = !self.consulta.is_empty();
        let texto_busqueda = if escribiendo {
            text(self.consulta.clone()).size(15).color(tema::texto())
        } else {
            text("Buscar aplicaciones…").size(15).color(tema::TEXTO2)
        };
        // La lupa dice para qué es el campo sin gastar una palabra, y es lo que
        // permite que el borde deje de gritarlo.
        let lupa: PanelElement<'_> = match icono::propio("buscar") {
            Some(ic) => icono::ver_teñido_propio(
                &ic,
                18.0,
                if escribiendo {
                    tema::acento()
                } else {
                    tema::TEXTO2
                },
            ),
            None => Space::new().into(),
        };
        // Radio 14 —el de los inputs (§2.3)— y no píldora, y el borde de acento
        // solo cuando hay algo escrito: el campo no se puede desenfocar, así
        // que un borde de acento permanente hace que el azul deje de significar
        // «esto está activo», que es lo único que el sistema le deja significar.
        let mut fila = row![lupa, texto_busqueda]
            .spacing(10)
            .align_y(iced_core::alignment::Vertical::Center);
        if escribiendo {
            // La ✕ solo aparece con algo escrito: es lo único que puede
            // limpiar, y vacía no tendría nada que hacer.
            fila = fila.push(Space::new().width(Length::Fill));
            if let Some(ic) = icono::propio("cerrar") {
                fila = fila.push(icono::ver_teñido_propio(&ic, 14.0, tema::TEXTO2));
            }
        }
        let busqueda = container(
            container(fila)
                .padding([0, 16])
                .height(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fixed(BUSQUEDA_ANCHO))
        .height(Length::Fixed(BUSQUEDA))
        .style(move |_theme| container::Style {
            background: Some(
                tema::alfa(tema::tinta(), if escribiendo { 0.14 } else { 0.10 }).into(),
            ),
            border: Border {
                radius: tema::R_CONTROL.into(),
                width: if escribiendo { 2.0 } else { 1.0 },
                color: if escribiendo {
                    tema::acento()
                } else {
                    tema::alfa(tema::tinta(), 0.16)
                },
            },
            ..Default::default()
        });

        let contenido = column![
            container(busqueda)
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            Space::new().height(Length::Fixed(HUECO)),
            self.rejilla_view(),
            self.puntos_view(),
        ]
        .align_x(Horizontal::Center);

        // Sin fondo: el velo lo pone el compositor a pantalla completa. Este
        // buffer es transparente salvo donde hay algo dibujado.
        container(contenido)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// El icono de una aplicación, sin rótulo. Está aparte de [`Self::celda_view`]
    /// porque el arrastre lo pinta también, en su propia superficie.
    fn baldosa_app(&self, indice_app: usize, px: f32, alfa: f32) -> PanelElement<'_> {
        match self.iconos.get(&indice_app).and_then(Option::as_ref) {
            Some(Icono::Svg(handle)) => svg(handle.clone())
                .width(Length::Fixed(px))
                .height(Length::Fixed(px))
                .opacity(alfa)
                .into(),
            Some(Icono::Raster(handle)) => iced_image(handle.clone())
                .width(Length::Fixed(px))
                .height(Length::Fixed(px))
                .opacity(alfa)
                .into(),
            // Baldosa con la inicial, igual que en el dock: un icono que falta
            // no puede dejar un hueco en la rejilla.
            None => {
                let inicial = self.apps[indice_app]
                    .nombre
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                container(text(inicial).size(px * 0.42).color(tema::texto()))
                    .width(Length::Fixed(px))
                    .height(Length::Fixed(px))
                    .center_x(Length::Fixed(px))
                    .center_y(Length::Fixed(px))
                    .style(move |_theme| container::Style {
                        background: Some(
                            Color {
                                a: 0.25,
                                ..tema::acento()
                            }
                            .into(),
                        ),
                        border: Border {
                            // Círculo, no baldosa redondeada: el sistema de
                            // diseño define los iconos de aplicación como un
                            // círculo de color con un glifo blanco encima
                            // (§3.1), así que el respaldo de uno que falta
                            // tiene que tener la misma silueta que sus vecinos.
                            radius: tema::R_PILL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into()
            }
        }
    }

    fn celda_view(&self, indice: usize) -> PanelElement<'_> {
        let (cw, ch) = (self.celda_ancho(), self.celda_alto());
        let hueco = || -> PanelElement<'_> {
            Space::new()
                .width(Length::Fixed(cw))
                .height(Length::Fixed(ch))
                .into()
        };
        let Some(item) = self.visibles.get(indice).copied() else {
            return hueco();
        };
        let px = self.icono_px();
        let alfa = 1.0;

        // La carpeta tiene su propia baldosa —la rejilla de miniaturas— y su
        // propio rótulo; el resto de la celda es idéntico.
        let (indice_app, nombre) = match item {
            Item::App(a) => (a, self.apps[a].nombre.clone()),
            Item::Carpeta(c) => {
                let interior = column![
                    self.tapa_carpeta(c, px, alfa),
                    Space::new().height(Length::Fixed(8.0)),
                    self.rotulo(&self.carpetas[c].nombre, cw, alfa),
                ]
                .align_x(Horizontal::Center);
                return container(interior)
                    .width(Length::Fixed(cw))
                    .height(Length::Fixed(ch))
                    .center_x(Length::Fixed(cw))
                    .center_y(Length::Fixed(ch))
                    .into();
            }
        };
        let app = &self.apps[indice_app];
        let _ = &nombre;
        // Editando, el icono baja al 88 % dentro de la misma celda: la ✕ va en
        // su esquina y sin encoger le tapa el dibujo. La celda no cambia de
        // tamaño, así que la etiqueta no salta al entrar y salir del modo.
        let px_icono = if self.editando { px * 0.88 } else { px };
        let baldosa = container(self.baldosa_app(indice_app, px_icono, alfa))
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .center_x(Length::Fixed(px))
            .center_y(Length::Fixed(px))
            .into();

        let etiqueta = self.rotulo(&app.nombre, cw, alfa);

        // En modo edición, la ✕ va **encima** del icono, en su esquina: apilada
        // y no en la columna, que si no empujaría la etiqueta.
        let baldosa: PanelElement<'_> = if self.editando {
            iced_widget::stack![baldosa, self.cierre_view(px)].into()
        } else {
            baldosa
        };
        let interior = column![baldosa, Space::new().height(Length::Fixed(8.0)), etiqueta]
            .align_x(Horizontal::Center);

        // El realce de la celda señalada **no** se dibuja aquí: va en su propia
        // superficie, encima del velo y debajo de esta. Ver `realce`.
        container(interior)
            .width(Length::Fixed(cw))
            .height(Length::Fixed(ch))
            .center_x(Length::Fixed(cw))
            .center_y(Length::Fixed(ch))
            .into()
    }

    /// La rejilla de la página actual.
    fn rejilla_view(&self) -> PanelElement<'_> {
        let mut rejilla = column![];
        // Dentro de una carpeta se dibujan solo las filas que ocupa lo que hay:
        // el resto son celdas vacías que estirarían el panel.
        let filas = match self.dentro {
            Some(_) => self.filas_panel(),
            None => self.filas(),
        };
        for fila in 0..filas {
            let mut linea = row![];
            let hueco = self.hueco_fila(fila);
            if hueco > 0.0 {
                linea = linea.push(Space::new().width(Length::Fixed(hueco)));
            }
            for col in 0..self.columnas() {
                let en_pagina = fila * self.columnas() + col;
                linea = linea.push(self.celda_view(self.pagina * self.por_pagina() + en_pagina));
            }
            rejilla = rejilla.push(linea);
        }
        rejilla.into()
    }

    /// La vista de dentro de una carpeta: el panel con su nombre y su rejilla.
    ///
    /// El panel es un **diálogo** —radio 26 y sombra de modal— teñido del color
    /// de la carpeta. Antes esto reusaba la rejilla de 6×5 entera y una fila de
    /// diez colores suelta bajo el título: ocho aplicaciones repartidas en
    /// treinta huecos no se leen como un grupo, y los diez puntos pesaban más
    /// que el nombre.
    fn view_carpeta(&self, carpeta: usize) -> PanelElement<'_> {
        let panel = self.rect_panel();
        // Editando se enseña el cursor con el mismo carácter que la vista de
        // escritorios, para que renombrar se vea igual en los dos sitios.
        let (nombre, pista) = match &self.renombrando {
            Some(escrito) => (format!("{escrito}│"), "Intro para guardar".to_string()),
            None => (
                self.carpetas[carpeta].nombre.clone(),
                match self.apps_de(carpeta).len() {
                    1 => "1 aplicación".to_string(),
                    n => format!("{n} aplicaciones"),
                },
            ),
        };

        // Cabecera: el nombre es a la vez el campo —se pulsa y se escribe— y a
        // la derecha el chip del color. Sin botón de cerrar: se sale con Esc o
        // pulsando fuera, igual que se cierra el launchpad entero.
        let cabecera = row![
            column![
                text(nombre).size(20).color(tema::texto()),
                text(pista).size(11).color(tema::TEXTO2),
            ],
            Space::new().width(Length::Fill),
            self.chip_color_view(carpeta),
        ]
        .align_y(iced_core::alignment::Vertical::Center);

        let (fondo, borde) = self.tinte_panel(carpeta);
        let contenido = column![
            container(cabecera)
                .width(Length::Fill)
                .height(Length::Fixed(PANEL_CABECERA))
                .center_y(Length::Fixed(PANEL_CABECERA)),
            self.rejilla_view(),
        ];
        // Los puntos solo si hay más de una página: una carpeta de ocho
        // aplicaciones no tiene de qué informar.
        let contenido = if self.paginas() > 1 {
            contenido.push(self.puntos_view())
        } else {
            contenido
        };
        let tarjeta = container(contenido)
            .width(Length::Fixed(panel.width))
            .height(Length::Fixed(panel.height))
            .padding(PANEL_PAD)
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: borde,
                },
                // La sombra de modal de §2.5. Es la única que se dibuja en todo
                // el launchpad: en oscuro las tarjetas no llevan, pero esto no
                // es una tarjeta fija, es un diálogo sobre el velo.
                shadow: tema::sombra_modal(),
                ..Default::default()
            });

        // El panel va colocado a mano y no centrado por el layout: sus
        // coordenadas tienen que ser **las mismas** que las de `rect_panel`,
        // que es lo que usa el hit-test del chip y de la rejilla de dentro.
        let colocado = column![
            Space::new().height(Length::Fixed(panel.y)),
            row![Space::new().width(Length::Fixed(panel.x)), tarjeta],
        ];

        let contenido: PanelElement<'_> = if self.paleta_abierta {
            let hueco = self.rect_paleta();
            iced_widget::stack![
                colocado,
                column![
                    Space::new().height(Length::Fixed(hueco.y)),
                    row![
                        Space::new().width(Length::Fixed(hueco.x)),
                        self.paleta_view(carpeta),
                    ],
                ],
            ]
            .into()
        } else {
            colocado.into()
        };

        container(contenido)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// El relleno y el filo del panel de una carpeta, según su color.
    ///
    /// El tinte va **sobre** la tarjeta del sistema y no en vez de ella: el
    /// color que el usuario le puso tiene que reconocerse al abrirla, pero el
    /// fondo sigue siendo `card`.
    fn tinte_panel(&self, carpeta: usize) -> (Color, Color) {
        match self.carpetas[carpeta].color.map(ColorCarpeta::color) {
            Some(color) => (
                tema::mezclar(tema::card(), color, 0.26),
                tema::alfa(color, 0.55),
            ),
            None => (tema::card(), tema::alfa(tema::tinta(), 0.10)),
        }
    }

    /// El chip cerrado del selector: un círculo con el color puesto.
    ///
    /// Sin color es un círculo hueco con el filo de la tinta, no un punto gris
    /// relleno: relleno se lee como un color más —el gris— y no como «esta
    /// carpeta no tiene».
    fn chip_color_view(&self, carpeta: usize) -> PanelElement<'_> {
        let puesto = self.carpetas[carpeta].color.map(ColorCarpeta::color);
        let abierta = self.paleta_abierta;
        container(
            container(Space::new())
                .width(Length::Fixed(COLOR_PUNTO))
                .height(Length::Fixed(COLOR_PUNTO))
                .style(move |_theme| container::Style {
                    background: puesto.map(Into::into),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        width: if puesto.is_some() { 0.0 } else { 1.5 },
                        color: tema::alfa(tema::tinta(), 0.35),
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(CHIP_COLOR))
        .height(Length::Fixed(CHIP_COLOR))
        .center_x(Length::Fixed(CHIP_COLOR))
        .center_y(Length::Fixed(CHIP_COLOR))
        // Desplegado, el chip lleva anillo: es lo que dice que el popover de
        // debajo sale de aquí y no de cualquier sitio.
        .style(move |_theme| container::Style {
            border: Border {
                radius: tema::R_PILL.into(),
                width: if abierta { 2.0 } else { 0.0 },
                color: tema::alfa(tema::tinta(), 0.5),
            },
            ..Default::default()
        })
        .into()
    }

    /// El popover del selector: los diez acentos y la tira de tono libre.
    ///
    /// Los diez van primero porque son pares calibrados; la tira está debajo
    /// para quien quiera el suyo. Lo que se elige ahí es el **tono**: la
    /// saturación y la luminosidad las pone [`de_tono`], que es lo que impide
    /// que un color libre se salga del contraste del resto de la paleta.
    fn paleta_view(&self, carpeta: usize) -> PanelElement<'_> {
        let puesto = self.carpetas[carpeta].color;
        let rect = self.rect_paleta();

        let mut acentos = row![].spacing(COLOR_PASO - COLOR_SLOT);
        for acento in tema::Acento::FIJOS {
            acentos = acentos.push(
                self.punto_color_view(acento.color(), puesto == Some(ColorCarpeta::Acento(acento))),
            );
        }

        // La tira es un degradado hecho a trozos: iced pinta gradientes
        // lineales, pero uno de 360° pasa por seis paradas y sale más código
        // que treinta y seis rectángulos de color plano.
        let mut tira = row![];
        for i in 0..TONO_TROZOS {
            let grados = i as f32 * 360.0 / TONO_TROZOS as f32;
            let color = de_tono(grados);
            // Las esquinas solo en los extremos, para que la tira entera tenga
            // forma de píldora sin dibujar una máscara.
            let radios = iced_core::border::Radius::default()
                .top_left(if i == 0 { TONO_ALTO / 2.0 } else { 0.0 })
                .bottom_left(if i == 0 { TONO_ALTO / 2.0 } else { 0.0 })
                .top_right(if i + 1 == TONO_TROZOS {
                    TONO_ALTO / 2.0
                } else {
                    0.0
                })
                .bottom_right(if i + 1 == TONO_TROZOS {
                    TONO_ALTO / 2.0
                } else {
                    0.0
                });
            tira = tira.push(
                container(Space::new())
                    .width(Length::Fixed(TONO_ANCHO / TONO_TROZOS as f32))
                    .height(Length::Fixed(TONO_ALTO))
                    .style(move |_theme| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            radius: radios,
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }

        // El tono elegido lleva su marca encima de la tira, en su sitio: sin
        // ella, un color libre puesto no se distingue de ninguno puesto.
        let tira: PanelElement<'_> = match puesto {
            Some(ColorCarpeta::Tono(grados)) => {
                let x = (grados / 360.0 * TONO_ANCHO - TONO_ALTO / 2.0).max(0.0);
                iced_widget::stack![
                    tira,
                    row![
                        Space::new().width(Length::Fixed(x)),
                        container(Space::new())
                            .width(Length::Fixed(TONO_ALTO))
                            .height(Length::Fixed(TONO_ALTO))
                            .style(|_theme| container::Style {
                                border: Border {
                                    radius: tema::R_PILL.into(),
                                    width: 2.5,
                                    color: tema::texto(),
                                },
                                ..Default::default()
                            }),
                    ],
                ]
                .into()
            }
            _ => tira.into(),
        };

        container(column![acentos, tira].spacing(PALETA_PAD))
            .width(Length::Fixed(rect.width))
            .height(Length::Fixed(rect.height))
            .padding(PALETA_PAD)
            .style(|_theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: 1.0,
                    color: tema::alfa(tema::tinta(), 0.09),
                },
                shadow: tema::sombra_popover(),
                ..Default::default()
            })
            .into()
    }

    /// Un punto de color del selector, con su anillo si es el puesto.
    fn punto_color_view<'a>(&self, color: Color, elegido: bool) -> PanelElement<'a> {
        container(
            container(Space::new())
                .width(Length::Fixed(COLOR_PUNTO))
                .height(Length::Fixed(COLOR_PUNTO))
                .style(move |_theme| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(COLOR_SLOT))
        .height(Length::Fixed(COLOR_SLOT))
        .center_x(Length::Fixed(COLOR_SLOT))
        .center_y(Length::Fixed(COLOR_SLOT))
        .style(move |_theme| container::Style {
            border: Border {
                radius: tema::R_PILL.into(),
                width: if elegido { 2.0 } else { 0.0 },
                color: tema::texto(),
            },
            ..Default::default()
        })
        .into()
    }

    /// La ✕ que quita una aplicación de la rejilla, en la esquina de su icono.
    fn cierre_view<'a>(&self, px: f32) -> PanelElement<'a> {
        let aspa: PanelElement<'a> = match crate::icono::propio("cerrar") {
            Some(ic) => {
                crate::icono::ver_teñido_propio(&ic, CIERRE * 0.5, tema::tinta_sobre(tema::rojo()))
            }
            None => Space::new().into(),
        };
        // El círculo es **rojo** y no gris: quitar algo de la rejilla es lo
        // único destructivo que se puede hacer aquí, y el sistema de diseño
        // reserva el rojo para eso.
        let boton = container(aspa)
            .width(Length::Fixed(CIERRE))
            .height(Length::Fixed(CIERRE))
            .center_x(Length::Fixed(CIERRE))
            .center_y(Length::Fixed(CIERRE))
            .style(|_theme| container::Style {
                background: Some(tema::rojo().into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        // Se coloca en la esquina superior izquierda del icono, saliéndose un
        // tercio: es donde la pone `cierre_de` para el hit-test.
        container(boton)
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .padding(iced_core::Padding::ZERO)
            .align_x(Horizontal::Left)
            .align_y(iced_core::alignment::Vertical::Top)
            .into()
    }

    /// El rótulo bajo un icono, que es el mismo para aplicaciones y carpetas.
    fn rotulo<'a>(&self, nombre: &str, cw: f32, alfa: f32) -> PanelElement<'a> {
        container(
            text(nombre.to_string())
                .size(UNIDAD * 0.75)
                .color(Color {
                    a: alfa,
                    ..tema::texto()
                })
                .align_x(Horizontal::Center),
        )
        .width(Length::Fixed(cw - UNIDAD))
        .height(Length::Fixed(ETIQUETA))
        .center_x(Length::Fixed(cw - UNIDAD))
        // Recortado a las dos líneas que la celda reserva. «KDE Partition
        // Manager» ocupa tres y sin recorte se metía en la fila de abajo,
        // encima de la etiqueta del icono siguiente: dos nombres pisados no se
        // leen ninguno de los dos.
        .clip(true)
        .into()
    }

    /// La tapa de una carpeta: sus nueve primeras aplicaciones en miniatura,
    /// dentro de una caja redondeada.
    ///
    /// Nueve y en rejilla de tres porque es lo que se reconoce como carpeta de
    /// un vistazo. Las miniaturas son los mismos iconos ya resueltos —no hay
    /// que cargar nada más— a un tercio del tamaño.
    fn tapa_carpeta<'a>(&'a self, carpeta: usize, px: f32, alfa: f32) -> PanelElement<'a> {
        let dentro = self.apps_de(carpeta);
        // Con color, la caja se tiñe de él y le sale un filo del mismo tono:
        // el relleno solo ya se pierde contra el velo, y el filo es lo que la
        // hace reconocible de una página a otra.
        let (fondo_tapa, borde_tapa) = match self.carpetas[carpeta].color.map(ColorCarpeta::color) {
            Some(color) => (tema::alfa(color, 0.28 * alfa), tema::alfa(color, alfa)),
            None => (tema::alfa(tema::tinta(), 0.18 * alfa), Color::TRANSPARENT),
        };
        let hueco = px * 0.08;
        let mini = (px - hueco * 3.0) / 2.0;
        // Con más de cuatro dentro, la cuarta casilla es la cuenta de lo que no
        // se ve: la novena aplicación de una carpeta nunca fue reconocible a
        // 20 px, y el número sí dice algo.
        let de_mas = dentro.len().saturating_sub(MINIATURAS - 1);
        let cuantas = if de_mas > 1 {
            MINIATURAS - 1
        } else {
            MINIATURAS
        };
        let mut rejilla = column![].spacing(hueco);
        for fila in 0..2 {
            let mut linea = row![].spacing(hueco);
            for col in 0..2 {
                let i = fila * 2 + col;
                if i >= cuantas {
                    let texto = format!("+{de_mas}");
                    linea = linea.push(
                        container(
                            text(texto)
                                .size(mini * 0.42)
                                .color(tema::alfa(tema::texto(), alfa)),
                        )
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .center_x(Length::Fixed(mini))
                        .center_y(Length::Fixed(mini))
                        .style(move |_theme| container::Style {
                            background: Some(tema::alfa(tema::tinta(), 0.14 * alfa).into()),
                            border: Border {
                                radius: (mini * 0.22).into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    );
                    continue;
                }
                let celda: PanelElement<'a> = match dentro
                    .get(i)
                    .and_then(|a| self.iconos.get(a))
                    .and_then(Option::as_ref)
                {
                    Some(Icono::Svg(handle)) => svg(handle.clone())
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .opacity(alfa)
                        .into(),
                    Some(Icono::Raster(handle)) => iced_image(handle.clone())
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .opacity(alfa)
                        .into(),
                    // El hueco se dibuja igual: una carpeta con dos
                    // aplicaciones tiene que verse como una carpeta, no como
                    // dos iconos pequeños sueltos.
                    None => Space::new()
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .into(),
                };
                linea = linea.push(celda);
            }
            rejilla = rejilla.push(linea);
        }
        container(rejilla)
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .center_x(Length::Fixed(px))
            .center_y(Length::Fixed(px))
            .style(move |_theme| container::Style {
                background: Some(fondo_tapa.into()),
                border: Border {
                    // Caja redondeada con el radio de la placa del pack de
                    // BookOS: `rx 80` sobre un lienzo de 512, el 15,6 % del
                    // lado. La tapa tiene que leerse como de la misma familia
                    // que lo que guarda, y con 0,22 —lo que había— se veía
                    // más redonda que los iconos de dentro.
                    radius: (px * 0.156).into(),
                    width: borde_tapa.a,
                    color: borde_tapa,
                },
                ..Default::default()
            })
            .into()
    }

    /// Los puntos de página: píldora de 20×8 el activo, círculo de 8 los demás.
    fn puntos_view(&self) -> PanelElement<'_> {
        let paginas = self.paginas();
        if paginas <= 1 {
            return Space::new().height(Length::Fixed(PUNTOS)).into();
        }
        let mut fila = row![].spacing(8);
        for i in 0..paginas {
            let activo = i == self.pagina;
            let ancho = if activo { 20.0 } else { 8.0 };
            fila = fila.push(
                container(
                    container(
                        Space::new()
                            .width(Length::Fixed(ancho))
                            .height(Length::Fixed(8.0)),
                    )
                    .style(move |_theme| container::Style {
                        background: Some(
                            if activo {
                                tema::acento()
                            } else {
                                Color {
                                    a: 0.35,
                                    ..tema::tinta()
                                }
                            }
                            .into(),
                        ),
                        border: Border {
                            radius: tema::R_PILL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                )
                // Ranura de ancho fijo: sin ella, la fila entera se recoloca al
                // cambiar de página porque el activo mide 12 px más.
                .width(Length::Fixed(20.0))
                .align_x(Horizontal::Center),
            );
        }
        container(fila)
            .width(Length::Fill)
            .height(Length::Fixed(PUNTOS))
            .center_x(Length::Fill)
            // Al fondo de la franja y no centrados: centrados quedaban a
            // 23 px de las etiquetas de la última fila y se leían como parte
            // de ella.
            .align_y(Vertical::Bottom)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arrastrar un icono hasta el dock lo fija, y lo fija **siempre**: la
    /// acción lleva `Some(true)` y no la alternancia del menú contextual, que
    /// soltando sobre algo ya anclado lo quitaría.
    #[test]
    fn soltar_un_icono_sobre_el_dock_lo_ancla() {
        let mut l = con_apps(&["Kate", "Konsole"]);
        l.poner_zona_dock(Some(iced_core::Rectangle {
            x: 600.0,
            y: 960.0,
            width: 400.0,
            height: 60.0,
        }));
        let celda = l.rect_celda(0).expect("la primera celda está en la página");
        l.pulsar(celda.center_x(), celda.center_y());
        l.puntero(Some((800.0, 990.0)));
        let (_, accion) = l.soltar();
        assert!(
            matches!(accion, Some(Accion::Anclar { fijar: Some(true), ref exec, .. }) if exec == "kate"),
            "se esperaba anclar Kate y salió {accion:?}"
        );
    }

    /// Sin dock a la vista, la misma franja es rejilla: soltar ahí saca la
    /// aplicación de su carpeta, que es lo que significa soltar fuera.
    #[test]
    fn sin_dock_soltar_abajo_no_ancla() {
        let mut l = con_apps(&["Kate", "Konsole"]);
        let celda = l.rect_celda(0).expect("la primera celda está en la página");
        l.pulsar(celda.center_x(), celda.center_y());
        l.puntero(Some((800.0, 990.0)));
        assert!(l.soltar().1.is_none());
    }

    /// Pulsar sobre el dock no agarra nada de la rejilla: ese clic no es suyo.
    #[test]
    fn pulsar_en_el_dock_no_agarra_la_rejilla() {
        let mut l = con_apps(&["Kate"]);
        l.poner_zona_dock(Some(iced_core::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1646.0,
            height: 1029.0,
        }));
        let celda = l.rect_celda(0).expect("la primera celda está en la página");
        l.pulsar(celda.center_x(), celda.center_y());
        assert!(!l.agarrado());
    }

    fn con_apps(nombres: &[&str]) -> Launchpad {
        con_apps_y_carpetas(nombres, Vec::new())
    }

    /// Un launchpad de mentira, sin tocar el disco: las aplicaciones se dan a
    /// mano y las carpetas también, que si no dependerían de lo que tenga el
    /// usuario en `~/.config`.
    fn con_apps_y_carpetas(nombres: &[&str], carpetas: Vec<Carpeta>) -> Launchpad {
        let apps: Vec<App> = nombres
            .iter()
            .map(|n| crate::apps::App {
                nombre: n.to_string(),
                exec: n.to_lowercase(),
                icono: n.to_lowercase(),
                normalizado: n.to_lowercase(),
            })
            .collect();
        let mut l = Launchpad {
            // Una pantalla cualquiera: los tests miran la lógica de búsqueda y
            // de páginas, que no depende del tamaño.
            pantalla: (1646.0, 1029.0),
            apps,
            carpetas,
            ocultas: Vec::new(),
            orden: Vec::new(),
            usos: HashMap::new(),
            editando: false,
            dentro: None,
            paleta_abierta: false,
            foco_paleta: None,
            renombrando: None,
            visibles: Vec::new(),
            consulta: String::new(),
            pagina: 0,
            seleccion: None,
            acumulado: 0.0,
            gesto_gastado: None,
            paso_por_borde: None,
            arrastre: None,
            zona_dock: None,
            transicion: None,
            tinte_fondo: tinte_del_fondo(),
            iconos: HashMap::new(),
        };
        l.rehacer_visibles();
        l
    }

    /// Manda la configuración a un directorio de usar y tirar.
    ///
    /// Crear o deshacer una carpeta **escribe en el disco**, y sin esto los
    /// tests se llevarían por delante el `launchpad.conf` de quien compile. Se
    /// hace una sola vez por proceso: `set_var` es del proceso entero y los
    /// tests corren en hilos a la vez.
    fn sin_tocar_el_disco() {
        static UNA_VEZ: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        UNA_VEZ.get_or_init(|| {
            let dir = std::env::temp_dir().join("bookos-tests-launchpad");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("directorio de pruebas");
            // SAFETY: se hace una vez, antes de que ningún test lea la
            // configuración, y ningún otro hilo toca esta variable.
            unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        });
    }

    /// Arrastrar una aplicación sobre otra crea una carpeta con las dos, y
    /// las dos dejan de estar sueltas en la rejilla.
    #[test]
    fn arrastrar_una_sobre_otra_crea_carpeta() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        assert_eq!(l.visibles.len(), 3);

        // Se agarra la primera y se suelta sobre el icono de la segunda.
        let origen = l.rect_celda(0).expect("la celda 0 se ve");
        let destino = l.rect_celda(1).expect("la celda 1 se ve");
        l.pulsar(origen.x + 10.0, origen.y + 10.0);
        l.puntero(Some((destino.center_x(), destino.y + 10.0)));
        let (repintar, accion) = l.soltar();
        assert!(repintar && accion.is_none(), "arrastrar no lanza nada");

        assert_eq!(l.carpetas.len(), 1);
        assert_eq!(l.carpetas[0].apps, ["kate", "firefox"]);
        // Una carpeta y la aplicación que quedó fuera.
        assert_eq!(l.visibles.len(), 2);
        assert_eq!(l.visibles[0], Item::Carpeta(0), "las carpetas van primero");
    }

    /// Soltar en el hueco de al lado de un icono lo recoloca, no crea carpeta,
    /// y el orden se queda para la próxima vez.
    #[test]
    fn soltar_en_el_hueco_recoloca() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        let origen = l.rect_celda(0).expect("la celda 0 se ve");
        let destino = l.rect_celda(2).expect("la celda 2 se ve");
        l.pulsar(origen.center_x(), origen.center_y());
        // El lado derecho de la última: detrás de Konsole.
        l.puntero(Some((destino.x + destino.width - 2.0, destino.center_y())));
        l.soltar();

        assert!(l.carpetas.is_empty(), "en el hueco no nace carpeta");
        assert_eq!(l.primeros(3), ["Kate", "Konsole", "Firefox"]);
        assert_eq!(l.orden, ["kate", "konsole", "firefox"]);
    }

    /// Sacando el icono por un lado de la rejilla se pasa de página, una sola vez aunque se
    /// siga empujando, y al soltar en una celda vacía el icono va al final.
    #[test]
    fn arrastrar_al_borde_lleva_a_otra_pagina() {
        sin_tocar_el_disco();
        let nombres: Vec<String> = (0..45).map(|i| format!("App{i:02}")).collect();
        let refs: Vec<&str> = nombres.iter().map(String::as_str).collect();
        let mut l = con_apps(&refs);
        let origen = l.rect_celda(0).expect("la celda 0 se ve");
        l.pulsar(origen.center_x(), origen.center_y());
        let derecha = l.size().0 + 10.0;
        assert!(l.puntero(Some((derecha, origen.center_y()))));
        assert_eq!(l.pagina, 1);
        l.puntero(Some((derecha + 1.0, origen.center_y())));
        assert_eq!(l.pagina, 1, "seguir fuera no encadena páginas");

        // La celda 44 es la última ocupada; la 50 está vacía.
        let vacia = l.rect_celda(50).expect("la celda 50 es de la página 1");
        l.puntero(Some((vacia.center_x(), vacia.center_y())));
        l.soltar();
        assert_eq!(l.pagina, 1);
        assert_eq!(l.primeros(45).last().map(String::as_str), Some("App00"));
    }

    /// Pulsar sin mover **sí** lanza: el arrastre no puede robarle el clic a
    /// quien solo quería abrir algo.
    #[test]
    fn pulsar_sin_mover_lanza() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox"]);
        let celda = l.rect_celda(0).unwrap();
        assert_eq!(
            l.pulsar(celda.x + 10.0, celda.y + 10.0),
            None,
            "pulsar agarra"
        );
        // Un temblor de dos píxeles sigue siendo un clic.
        l.puntero(Some((celda.x + 12.0, celda.y + 11.0)));
        let (_, accion) = l.soltar();
        assert_eq!(accion, Some(Accion::Lanzar("firefox".into())));
    }

    /// Y dentro de una carpeta, soltar fuera de la rejilla saca la aplicación;
    /// al quedarse con una sola, la carpeta se deshace.
    #[test]
    fn sacar_de_la_carpeta_la_deshace_al_quedarse_sola() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into(), "firefox".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        assert_eq!(l.visibles, [Item::Carpeta(0)], "las tres están dentro");

        // Se entra en la carpeta y se saca una arrastrándola arriba del todo,
        // fuera de la rejilla.
        l.abrir_carpeta(0);
        assert!(l.en_carpeta());
        assert_eq!(l.visibles.len(), 3);
        let celda = l.rect_celda(0).unwrap();
        l.pulsar(celda.x + 10.0, celda.y + 10.0);
        l.puntero(Some((celda.x + 10.0, 2.0)));
        l.soltar();
        assert_eq!(l.carpetas[0].apps.len(), 2, "sacó una");

        // Y al sacar la penúltima, la carpeta desaparece: una sola aplicación
        // no es una carpeta.
        let celda = l.rect_celda(0).unwrap();
        l.pulsar(celda.x + 10.0, celda.y + 10.0);
        l.puntero(Some((celda.x + 10.0, 2.0)));
        l.soltar();
        assert!(l.carpetas.is_empty(), "la carpeta tenía que deshacerse");
        assert!(!l.en_carpeta(), "y sacarnos de ella");
        assert_eq!(l.visibles.len(), 3, "las tres vuelven a estar sueltas");
    }

    /// El selector nace cerrado: se despliega con su chip, se elige un color, y
    /// volver a pulsar el que está puesto devuelve la carpeta al gris.
    #[test]
    fn la_carpeta_se_tiñe_y_se_destiñe() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        l.abrir_carpeta(0);

        // Cerrado, pulsar donde estarían los colores no elige nada: ahí no hay
        // nada que pulsar.
        let acentos = l.rect_acentos();
        l.pulsar(acentos.x + 2.0, acentos.center_y());
        assert_eq!(
            l.carpetas[0].color, None,
            "con el selector cerrado no hay colores"
        );

        let chip = l.rect_chip_color();
        l.pulsar(chip.center_x(), chip.center_y());
        assert!(l.paleta_abierta, "el chip despliega el selector");

        // El tercer punto de la fila es el tercer acento de la tabla.
        let x = acentos.x + COLOR_PASO * 2.0 + 2.0;
        l.pulsar(x, acentos.center_y());
        assert_eq!(
            l.carpetas[0].color,
            Some(ColorCarpeta::Acento(tema::Acento::TODOS[2]))
        );
        assert!(!l.paleta_abierta, "elegir un color cierra el selector");

        // Otra vez el mismo: vuelve al gris.
        l.pulsar(chip.center_x(), chip.center_y());
        l.pulsar(x, acentos.center_y());
        assert_eq!(l.carpetas[0].color, None);

        // Y la calle entre dos ranuras no elige nada, pero sí cierra: pulsar
        // fuera de un popover es lo que lo cierra.
        l.pulsar(chip.center_x(), chip.center_y());
        l.pulsar(acentos.x + COLOR_SLOT + 2.0, acentos.center_y());
        assert_eq!(l.carpetas[0].color, None);
        assert!(!l.paleta_abierta);
    }

    /// Entrar y salir de una carpeta deja apuntada su transición, y solo se
    /// puede recoger una vez: quien se la lleva es quien tiene la superficie de
    /// antes, y dos veces sería animar dos veces lo mismo.
    #[test]
    fn abrir_y_cerrar_una_carpeta_pide_su_transicion() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        assert_eq!(l.tomar_transicion(), None, "abrir el launchpad no es una");

        l.abrir_carpeta(0);
        assert_eq!(l.tomar_transicion(), Some(Transicion::AbrirCarpeta));
        assert_eq!(l.tomar_transicion(), None, "solo se recoge una vez");

        assert!(l.salir_de_carpeta());
        assert_eq!(l.tomar_transicion(), Some(Transicion::CerrarCarpeta));

        // Salir sin estar dentro no anima nada: no ha cambiado la vista.
        assert!(!l.salir_de_carpeta());
        assert_eq!(l.tomar_transicion(), None);
    }

    /// La tira da un color propio, y lo que se guarda es el **tono**: así el
    /// color de una carpeta sigue cambiando con el tema, como los acentos.
    #[test]
    fn la_tira_de_tono_da_un_color_propio_y_sobrevive_al_disco() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        l.abrir_carpeta(0);

        let chip = l.rect_chip_color();
        l.pulsar(chip.center_x(), chip.center_y());
        let tira = l.rect_tono();
        // Un cuarto de la tira son 90°, el verde amarillento.
        l.pulsar(tira.x + tira.width / 4.0, tira.center_y());
        let Some(ColorCarpeta::Tono(grados)) = l.carpetas[0].color else {
            panic!(
                "se esperaba un tono libre y salió {:?}",
                l.carpetas[0].color
            );
        };
        assert!(
            (grados - 90.0).abs() < 6.0,
            "{grados}° no son los 90 de un cuarto de tira"
        );

        // Ida y vuelta por el formato de `launchpad.conf`.
        let guardado = ColorCarpeta::Tono(grados).guardado();
        assert_eq!(
            ColorCarpeta::desde_guardado(&guardado),
            Some(ColorCarpeta::Tono(grados.round()))
        );
        // Y lo de siempre se sigue leyendo igual: un fichero escrito antes de
        // que existieran los tonos no puede perder los colores.
        assert_eq!(
            ColorCarpeta::desde_guardado("verde"),
            Some(ColorCarpeta::Acento(tema::Acento::Verde))
        );
    }

    /// La ✕ quita la aplicación de la rejilla —y de su carpeta— pero **no la
    /// desinstala**: sigue en el sistema y vuelve borrándola del fichero.
    #[test]
    fn la_equis_quita_de_la_rejilla_sin_desinstalar() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        assert_eq!(l.visibles.len(), 3);
        // Sin modo edición no hay ✕ que pulsar.
        assert!(l.cierre_de(0).is_none());

        l.alternar_edicion();
        let cierre = l.cierre_de(0).expect("con edición sí hay ✕");
        l.pulsar(cierre.x + 2.0, cierre.y + 2.0);
        assert_eq!(l.ocultas.len(), 1, "quitó una");
        assert_eq!(l.visibles.len(), 2);
        // Sigue instalada: lo que cambia es lo que se enseña.
        assert_eq!(l.apps.len(), 3);

        // Y tampoco sale buscándola, que si no la ✕ no habría servido de nada.
        l.tecla(crate::TeclaPulsada::Caracter(
            l.apps[0]
                .nombre
                .chars()
                .next()
                .unwrap()
                .to_ascii_lowercase(),
        ));
        assert!(
            !l.visibles.contains(&Item::App(0)),
            "la quitada vuelve a salir al buscar"
        );
    }

    /// Una carpeta nace llamándose «Carpeta»: hay que poder cambiarlo sin
    /// editar el fichero a mano.
    #[test]
    fn la_carpeta_se_renombra_desde_dentro() {
        use crate::TeclaPulsada as T;
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Carpeta".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        l.abrir_carpeta(0);

        // Pulsar el nombre lo pone a editar, y entonces el teclado es suyo:
        // escribir no busca.
        let titulo = l.rect_titulo();
        l.pulsar(titulo.x + 10.0, titulo.y + 10.0);
        assert!(l.renombrando.is_some());
        for c in "Trabajo".chars() {
            l.tecla(T::Caracter(c));
        }
        assert!(l.consulta.is_empty(), "escribir el nombre no puede buscar");
        assert!(matches!(l.tecla(T::Intro), Tecla::Consumida));
        assert_eq!(l.carpetas[0].nombre, "CarpetaTrabajo");
        assert!(l.en_carpeta(), "renombrar no saca de la carpeta");

        // Y Esc mientras se edita cancela sin tocar el nombre.
        l.pulsar(titulo.x + 10.0, titulo.y + 10.0);
        l.tecla(T::Retroceso);
        l.tecla(T::Escape);
        assert_eq!(l.carpetas[0].nombre, "CarpetaTrabajo");
        assert!(l.en_carpeta(), "el Esc de la edición no sale de la carpeta");
    }

    /// Buscando no hay carpetas: se busca entre todas las aplicaciones, estén
    /// dentro de una o no.
    #[test]
    fn la_busqueda_atraviesa_las_carpetas() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        assert_eq!(l.visibles.len(), 2, "la carpeta y Firefox");

        l.tecla(crate::TeclaPulsada::Caracter('k'));
        assert_eq!(l.visibles.len(), 2, "kate y konsole, sin la carpeta");
        assert!(l.visibles.iter().all(|i| matches!(i, Item::App(_))));

        // Y al borrar lo escrito vuelve la rejilla con su carpeta.
        l.tecla(crate::TeclaPulsada::Retroceso);
        assert_eq!(l.visibles[0], Item::Carpeta(0));
    }

    #[test]
    fn buscar_pone_delante_la_mas_usada() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["Vim", "VirtualBox", "Visor", "Navegador Vivaldi"]);
        l.usos = HashMap::from([("virtualbox".into(), 9), ("navegador vivaldi".into(), 50)]);
        l.tecla(T::Caracter('v'));
        l.tecla(T::Caracter('i'));
        assert_eq!(
            l.seleccionado(),
            Some(Accion::Lanzar("navegador vivaldi".into()))
        );
        l.tecla(T::Caracter('r'));
        assert_eq!(l.seleccionado(), Some(Accion::Lanzar("virtualbox".into())));
        // Lo difuso no adelanta a un prefijo por mucho que se use.
        l.usos.insert("visor".into(), 1);
        l.tecla(T::Retroceso);
        l.tecla(T::Retroceso);
        l.tecla(T::Caracter('i'));
        assert_eq!(
            l.seleccionado(),
            Some(Accion::Lanzar("navegador vivaldi".into()))
        );
    }

    #[test]
    fn escribir_filtra_y_preselecciona() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        assert_eq!(l.visibles.len(), 3);

        l.tecla(T::Caracter('k'));
        assert_eq!(l.visibles.len(), 2, "kate y konsole");
        assert_eq!(l.seleccion, Some(0), "con búsqueda, Intro tiene destino");

        l.tecla(T::Caracter('o'));
        assert_eq!(l.visibles.len(), 1, "solo konsole");
        assert_eq!(l.seleccionado(), Some(Accion::Lanzar("konsole".into())));

        // Retroceso deshace.
        l.tecla(T::Retroceso);
        assert_eq!(l.visibles.len(), 2);
    }

    #[test]
    fn escape_limpia_la_busqueda_antes_de_cerrar() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["Firefox"]);
        l.tecla(T::Caracter('f'));
        assert!(matches!(l.tecla(T::Escape), Tecla::Consumida));
        assert!(l.consulta.is_empty());
        // Ya sin búsqueda, el segundo Escape sí cierra.
        assert!(matches!(l.tecla(T::Escape), Tecla::Cerrar));
    }

    #[test]
    fn la_seleccion_arrastra_la_pagina() {
        use crate::TeclaPulsada as T;
        let nombres: Vec<String> = (0..45).map(|i| format!("App{i:02}")).collect();
        let refs: Vec<&str> = nombres.iter().map(String::as_str).collect();
        let mut l = con_apps(&refs);
        // La forma depende de la pantalla: se lee en vez de suponer el 6×5.
        let (columnas, filas) = l.forma();
        assert_eq!(l.paginas(), 2, "45 apps a {} por página", columnas * filas);

        // Bajar todas las filas desde el principio se sale de la primera página.
        for _ in 0..filas {
            l.tecla(T::Abajo);
        }
        assert_eq!(l.seleccion, Some(columnas * filas));
        assert_eq!(l.pagina, 1, "la selección arrastra la página");
    }

    #[test]
    fn el_gesto_pasa_de_pagina_con_umbral_y_espera() {
        let nombres: Vec<String> = (0..45).map(|i| format!("App{i:02}")).collect();
        let refs: Vec<&str> = nombres.iter().map(String::as_str).collect();
        let mut l = con_apps(&refs);
        assert_eq!(l.paginas(), 2);

        // Por debajo del umbral no pasa nada, pero se acumula.
        assert!(!l.desplazar(8.0, 0.0));
        assert!(!l.desplazar(8.0, 0.0));
        assert_eq!(l.pagina, 0);
        // Al superar los 20 px acumulados, cambia.
        assert!(l.desplazar(8.0, 0.0));
        assert_eq!(l.pagina, 1);
        assert!(
            !l.animando(),
            "cambiar de página no debe iniciar repintados completos por frame"
        );

        // Y justo después no encadena otro salto: un gesto de touchpad manda
        // decenas de eventos y sin la espera se recorrerían cinco páginas.
        assert!(!l.desplazar(40.0, 0.0));
        assert_eq!(l.pagina, 1);

        // Un gesto largo sigue siendo uno: cada evento alarga la espera, y
        // aunque el salto quede lejos, 100 ms desde el último evento no bastan.
        let hace = |ms| Some(std::time::Instant::now() - std::time::Duration::from_millis(ms));
        l.gesto_gastado = hace(100);
        assert!(!l.desplazar(-40.0, 0.0));
        assert_eq!(l.pagina, 1);

        // Tras levantar los dedos, el gesto siguiente pasa su página.
        l.gesto_gastado = hace(200);
        assert!(l.desplazar(-40.0, 0.0));
        assert_eq!(l.pagina, 0);
    }

    #[test]
    fn el_gesto_no_se_sale_por_los_extremos() {
        let mut l = con_apps(&["A", "B"]);
        // Una sola página: no hay a dónde ir ni hacia adelante ni hacia atrás.
        assert!(!l.desplazar(100.0, 0.0));
        assert!(!l.desplazar(-100.0, 0.0));
        assert_eq!(l.pagina, 0);
    }

    #[test]
    fn moverse_no_se_sale_de_la_lista() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["A", "B", "C"]);
        for _ in 0..10 {
            l.tecla(T::Derecha);
        }
        assert_eq!(l.seleccion, Some(2), "no pasa del último");
        for _ in 0..10 {
            l.tecla(T::Izquierda);
        }
        assert_eq!(l.seleccion, Some(0), "ni del primero");
    }
}
