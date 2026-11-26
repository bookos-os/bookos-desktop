//! Los iconos del escritorio: lo que hay en la carpeta Escritorio, dibujado
//! sobre el fondo y por debajo de las ventanas.
//!
//! ## Una superficie por icono, y no una del tamaño de la pantalla
//!
//! Lo natural sería una sola capa a pantalla completa con toda la rejilla
//! dentro. No vale: rasterizar un buffer de 2881×1801 con iced cuesta 23 ms
//! medidos (ver el comentario de `Shell::paint`), y la selección con banda
//! elástica cambia el aspecto de las celdas **en cada movimiento del ratón**.
//! Serían 40 ms por evento de puntero.
//!
//! Por eso cada icono es su propia superficie de 96×96 lógicos: al barrer con
//! la banda solo se repintan las celdas que **cambian** de estado —una o dos
//! por movimiento— y el resto de la escena la compone la GPU. La banda en sí no
//! se rasteriza aquí: son cinco rectángulos de color que dibuja el compositor.
//!
//! ## Qué hay dentro de la carpeta
//!
//! Se lee una vez al arrancar y se vuelve a mirar en el latido del panel —el
//! que ya despierta al cambiar el minuto—, así que guardar un fichero en el
//! escritorio se ve sin abrir ningún descriptor nuevo ni programar un
//! temporizador propio. El precio es hasta un minuto de retraso, que es el
//! trato que este escritorio hace siempre a cambio de no despertar de más.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use iced_core::alignment::Horizontal;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, text};

use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

/// Lado de una celda de la rejilla, en píxeles lógicos.
///
/// 96 es el ancho que deja caber dos renglones de nombre de 11 px sin que
/// «Documentos comprimidos.tar.gz» se coma a su vecino, y el alto que cabe
/// entero —icono, aire y etiqueta— sin recortar la segunda línea.
pub const CELDA: (f32, f32) = (96.0, 96.0);
/// Lado del icono dentro de la celda.
pub const ICONO: f32 = 48.0;
/// Aire entre el borde de la pantalla y la primera columna, y entre el panel y
/// la primera fila.
const MARGEN_X: f32 = 12.0;
const MARGEN_Y: f32 = 8.0;
/// Lo que se reserva abajo para que la última fila no quede debajo del dock.
///
/// El dock flota a 78 lógicos del canto inferior contando su margen; 96 deja
/// además el aire de una celda entre el nombre y el cristal.
const RESERVA_DOCK: f32 = 96.0;

/// Cuerpo del nombre. Once es lo que deja caber «informe-trimestral.pdf» en
/// dos renglones dentro de 80 px útiles.
const TAMANO_NOMBRE: f32 = 11.0;

/// Un icono del escritorio.
pub struct Elemento {
    pub nombre: String,
    pub ruta: PathBuf,
    /// Qué ejecutar al abrirlo: el `Exec` de su `.desktop`, o `xdg-open`.
    orden: String,
    icono: Option<Icono>,
    /// El color puesto a mano a una carpeta, el mismo que guarda
    /// bookos-explorer. `None` es el automático por nombre.
    color: Option<String>,
    /// Si es un acceso directo (un enlace simbólico) y si su original sigue ahí.
    // añadido en la 0.2.3
    pub acceso: Acceso,
    /// Columna y fila que ocupa. Es lo que se guarda en disco, no los píxeles:
    /// así una posición sobrevive a cambiar de resolución.
    celda: (i32, i32),
    seleccionado: bool,
}

/// Lo que ofrece el submenú «Crear» del clic derecho.
// 0.2.0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NuevoElemento {
    Carpeta,
    Texto,
    Script,
    Vacio,
}

impl NuevoElemento {
    /// Nombre base y extensión.
    fn nombre(self) -> (&'static str, &'static str) {
        match self {
            Self::Carpeta => (crate::emergente::t("Nueva carpeta", "New folder"), ""),
            Self::Texto => (
                crate::emergente::t("Nuevo documento de texto", "New text document"),
                ".txt",
            ),
            Self::Script => (crate::emergente::t("Nuevo script", "New script"), ".sh"),
            Self::Vacio => (crate::emergente::t("Nuevo archivo", "New file"), ""),
        }
    }

    /// Un script sin shebang no se ejecuta; los demás nacen vacíos.
    fn contenido(self) -> &'static str {
        match self {
            Self::Script => "#!/bin/sh\n",
            _ => "",
        }
    }
}

impl Elemento {
    // añadido en la 0.2.3
    pub fn icono(&self) -> Option<&Icono> {
        self.icono.as_ref()
    }

    /// Lo que se ejecuta al abrirlo.
    // añadido en la 0.2.3
    pub fn orden(&self) -> &str {
        &self.orden
    }
}

/// Un icono que es un enlace simbólico se dibuja con la marca de acceso
/// directo; si su original ya no está, apagado y con aviso.
// añadido en la 0.2.3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceso {
    No,
    Si,
    Roto,
}

/// Por qué se ordena con «Ordenar por».
// añadido en la 0.2.3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orden {
    Nombre,
    Tipo,
    Fecha,
    Tamano,
}

/// Tamaño de los iconos del escritorio. Escala la celda entera: un icono
/// grande en una celda de las de siempre se comería el nombre del vecino.
// añadido en la 0.2.3
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tamano {
    Pequeno,
    #[default]
    Mediano,
    Grande,
}

impl Tamano {
    // añadido en la 0.2.3
    pub fn escala(self) -> f32 {
        match self {
            // 0,8 deja la celda en 77 px: lo justo para dos renglones de 11
            // con «Documentos» entero; por debajo el nombre ya no cabe.
            Self::Pequeno => 0.8,
            Self::Mediano => 1.0,
            Self::Grande => 1.25,
        }
    }

    // añadido en la 0.2.3
    pub fn clave(self) -> &'static str {
        match self {
            Self::Pequeno => "pequeno",
            Self::Mediano => "mediano",
            Self::Grande => "grande",
        }
    }

    // añadido en la 0.2.3
    pub fn de_clave(clave: &str) -> Option<Self> {
        match clave {
            "pequeno" => Some(Self::Pequeno),
            "mediano" => Some(Self::Mediano),
            "grande" => Some(Self::Grande),
            _ => None,
        }
    }
}

pub struct Escritorio {
    elementos: Vec<Elemento>,
    // añadido en la 0.2.3
    tamano: Tamano,
    /// El último «Ordenar por», para marcarlo en el menú. Mover un icono a
    /// mano lo borra: ya no están en ese orden.
    // añadido en la 0.2.3
    orden: Option<Orden>,
    /// La zona de la rejilla en lógicos: `(x, y, ancho, alto)`. Empieza bajo el
    /// panel y acaba antes del dock.
    zona: (f32, f32, f32, f32),
    filas: i32,
    columnas: i32,
    /// Cuánto se han corrido los iconos seleccionados mientras se arrastran.
    /// Va aparte de sus celdas porque durante el arrastre no han cambiado de
    /// sitio todavía: solo se dibujan corridos.
    arrastre: Option<(f32, f32)>,
    /// Si se enseñan los iconos. Apagado, la lista se queda vacía: no hay
    /// superficies que componer ni nada que pulsar, y el clic cae al fondo.
    visible: bool,
}

impl Escritorio {
    /// `alto_panel` y `pantalla` van en píxeles lógicos.
    // 0.1.0
    // modified 0.1.1 -- puede nacer con los iconos ocultos
    // modified 0.2.3 -- con tamaño de icono
    pub fn new(alto_panel: f32, pantalla: (f32, f32), visible: bool, tamano: Tamano) -> Self {
        let mut escritorio = Self {
            elementos: if visible { leer() } else { Vec::new() },
            tamano,
            orden: None,
            zona: (0.0, 0.0, 0.0, 0.0),
            filas: 1,
            columnas: 1,
            arrastre: None,
            visible,
        };
        escritorio.recolocar(alto_panel, pantalla);
        escritorio
    }

    /// Rehace la rejilla para un tamaño de pantalla. Conserva la columna y la
    /// fila de cada icono mientras quepan; lo que se sale se recoloca al primer
    /// hueco libre, que es mejor que dejarlo fuera de la pantalla.
    // 0.1.0
    pub fn recolocar(&mut self, alto_panel: f32, (ancho, alto): (f32, f32)) {
        let celda = self.celda();
        self.zona = (
            MARGEN_X,
            alto_panel + MARGEN_Y,
            (ancho - MARGEN_X * 2.0).max(celda.0),
            (alto - alto_panel - MARGEN_Y - RESERVA_DOCK).max(celda.1),
        );
        self.columnas = (self.zona.2 / celda.0).floor().max(1.0) as i32;
        self.filas = (self.zona.3 / celda.1).floor().max(1.0) as i32;
        self.asignar_celdas();
    }

    /// Lado de la celda con el tamaño de icono elegido.
    // añadido en la 0.2.3
    pub fn celda(&self) -> (f32, f32) {
        let e = self.tamano.escala();
        (CELDA.0 * e, CELDA.1 * e)
    }

    // añadido en la 0.2.3
    pub fn tamano(&self) -> Tamano {
        self.tamano
    }

    /// Cambia el tamaño de los iconos. `true` si cambió. La rejilla se rehace
    /// conservando columna y fila de cada uno, como al cambiar de resolución.
    // añadido en la 0.2.3
    pub fn cambiar_tamano(&mut self, tamano: Tamano, alto_panel: f32, pantalla: (f32, f32)) -> bool {
        if self.tamano == tamano {
            return false;
        }
        self.tamano = tamano;
        self.recolocar(alto_panel, pantalla);
        true
    }

    /// Recoloca todos los iconos por columnas, de arriba abajo, en ese orden.
    /// Las carpetas van siempre primero, como en el explorador. La fecha y el
    /// tamaño van de mayor a menor: lo reciente y lo grande es lo que se busca.
    // añadido en la 0.2.3
    pub fn ordenar(&mut self, orden: Orden) {
        let clave_tipo = |e: &Elemento| {
            e.ruta
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        let meta = |e: &Elemento| std::fs::metadata(&e.ruta).ok();
        self.elementos.sort_by(|a, b| {
            let carpetas = b.ruta.is_dir().cmp(&a.ruta.is_dir());
            let nombre = || a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase());
            carpetas.then_with(|| match orden {
                Orden::Nombre => nombre(),
                Orden::Tipo => clave_tipo(a).cmp(&clave_tipo(b)).then_with(nombre),
                Orden::Fecha => {
                    let f = |e| meta(e).and_then(|m| m.modified().ok());
                    f(b).cmp(&f(a)).then_with(nombre)
                }
                Orden::Tamano => {
                    let t = |e| meta(e).map_or(0, |m| m.len());
                    t(b).cmp(&t(a)).then_with(nombre)
                }
            })
        });
        let mut ocupadas = Vec::new();
        for elemento in &mut self.elementos {
            elemento.celda = primer_hueco(&ocupadas, self.columnas, self.filas);
            ocupadas.push(elemento.celda);
        }
        self.orden = Some(orden);
        self.guardar_posiciones();
    }

    // añadido en la 0.2.3
    pub fn orden(&self) -> Option<Orden> {
        self.orden
    }

    /// Quita los huecos: cada icono pasa al primer sitio libre en el orden en
    /// que se ven ahora (por columnas). Es el «Limpiar» de macOS; aquí no hace
    /// falta encajar en la rejilla porque un icono nunca sale de ella.
    // añadido en la 0.2.3
    pub fn juntar(&mut self) {
        let mut orden: Vec<usize> = (0..self.elementos.len()).collect();
        orden.sort_by_key(|&i| self.elementos[i].celda);
        let mut ocupadas = Vec::new();
        for i in orden {
            let celda = primer_hueco(&ocupadas, self.columnas, self.filas);
            self.elementos[i].celda = celda;
            ocupadas.push(celda);
        }
        self.guardar_posiciones();
    }

    /// Renombra lo seleccionado (uno solo) y conserva su sitio. `Err` con el
    /// motivo en palabras, para decirlo en el diálogo.
    // añadido en la 0.2.3
    pub fn renombrar(&mut self, nuevo: &str) -> Result<(), &'static str> {
        let nuevo = nuevo.trim();
        if nuevo.is_empty() || nuevo == "." || nuevo == ".." || nuevo.contains('/') {
            return Err(crate::t("Ese nombre no vale", "That name isn't valid"));
        }
        let mut seleccion = self.elementos.iter_mut().filter(|e| e.seleccionado);
        let (Some(elemento), None) = (seleccion.next(), seleccion.next()) else {
            return Err(crate::t("Elige un solo elemento", "Select a single item"));
        };
        let destino = elemento.ruta.with_file_name(nuevo);
        if destino == elemento.ruta {
            return Ok(());
        }
        // `exists` sigue enlaces: uno roto con ese nombre también cuenta.
        if destino.exists() || std::fs::symlink_metadata(&destino).is_ok() {
            return Err(crate::t("Ya hay algo con ese nombre", "That name is taken"));
        }
        std::fs::rename(&elemento.ruta, &destino)
            .map_err(|_| crate::t("No se pudo renombrar", "Couldn't rename"))?;
        // Se cambia aquí y no releyendo: releer lo mandaría al primer hueco,
        // porque la posición guardada va por nombre.
        elemento.ruta = destino;
        elemento.nombre = nuevo.to_string();
        // Un lanzador sigue ejecutando su `Exec`; lo demás se abre por ruta.
        if elemento.ruta.extension().is_none_or(|e| e != "desktop") {
            elemento.orden = format!("xdg-open {}", entrecomillar(&elemento.ruta));
        }
        self.guardar_posiciones();
        Ok(())
    }

    /// Selecciona solo el que se llama así. Lo usa «Agrupar» para dejar
    /// lista la carpeta nueva.
    // añadido en la 0.2.3
    pub fn seleccionar_nombre(&mut self, nombre: &str) {
        for e in &mut self.elementos {
            e.seleccionado = e.nombre == nombre;
        }
    }

    /// Lo seleccionado, en orden.
    // añadido en la 0.2.3
    pub fn seleccionados(&self) -> impl Iterator<Item = &Elemento> {
        self.elementos.iter().filter(|e| e.seleccionado)
    }

    /// Reparte las celdas: primero las guardadas que siguen cabiendo, y el
    /// resto al primer hueco libre, llenando por columnas de arriba abajo.
    // 0.1.0
    fn asignar_celdas(&mut self) {
        let guardadas = posiciones_guardadas();
        let mut ocupadas: Vec<(i32, i32)> = Vec::new();
        // Dos vueltas: primero se apalabran las posiciones guardadas, para que
        // un icono nuevo no le quite el sitio a uno que el usuario colocó.
        for elemento in &mut self.elementos {
            elemento.celda = (-1, -1);
            if let Some(&celda) = guardadas
                .iter()
                .find(|(nombre, _)| *nombre == elemento.nombre)
                .map(|(_, celda)| celda)
                && celda.0 < self.columnas
                && celda.1 < self.filas
                && !ocupadas.contains(&celda)
            {
                elemento.celda = celda;
                ocupadas.push(celda);
            }
        }
        for elemento in &mut self.elementos {
            if elemento.celda != (-1, -1) {
                continue;
            }
            elemento.celda = primer_hueco(&ocupadas, self.columnas, self.filas);
            ocupadas.push(elemento.celda);
        }
    }

    /// Vuelve a mirar la carpeta. `true` si algo cambió y hay que repintar.
    ///
    /// Compara nombres y no marcas de tiempo a propósito: lo que se ve en el
    /// escritorio es la lista, y `mtime` del directorio cambia también cuando
    /// se toca un fichero que ya estaba.
    // 0.1.0
    // modified 0.1.1 -- no relee con los iconos ocultos
    pub fn releer(&mut self, alto_panel: f32, pantalla: (f32, f32)) -> bool {
        if !self.visible {
            return false;
        }
        let nuevos = leer();
        let iguales = nuevos.len() == self.elementos.len()
            && nuevos
                .iter()
                .zip(&self.elementos)
                .all(|(a, b)| a.ruta == b.ruta);
        if iguales {
            return false;
        }
        // La selección se pierde al releer. Es correcto: lo seleccionado puede
        // ser justo lo que acaba de desaparecer, y arrastrar de una selección
        // fantasma borraría o movería otra cosa.
        self.elementos = nuevos;
        self.recolocar(alto_panel, pantalla);
        true
    }

    /// Vuelve a resolver los iconos de los lanzadores `.desktop` del escritorio
    /// tras cambiar de tema o de pack de iconos.
    pub fn recargar_iconos(&mut self) -> bool {
        let mut cambio = false;
        for elemento in &mut self.elementos {
            if elemento.ruta.extension().is_some_and(|e| e == "desktop")
                && let Some(app) = crate::apps::leer_lanzador(&elemento.ruta)
            {
                elemento.icono = crate::icono::cargar(&app.icono)
                    .or_else(|| Some(icono_fichero(&elemento.ruta)));
                cambio = true;
            }
        }
        cambio
    }

    /// Crea una carpeta o un archivo vacío y lo deja seleccionado.
    ///
    /// Sin escotilla propia: si no hay `~/Escritorio` —ni siquiera se ha usado
    /// nunca el escritorio en esta cuenta— no hay dónde crear nada, y no es
    /// este el sitio para decidir que de repente sí la hay.
    // 0.2.0
    pub fn crear_nuevo(
        &mut self,
        tipo: NuevoElemento,
        alto_panel: f32,
        pantalla: (f32, f32),
    ) -> bool {
        let Some(carpeta) = carpeta_escritorio() else {
            tracing::warn!("sin carpeta de escritorio: no se puede crear nada en ella");
            return false;
        };
        let (base, extension) = tipo.nombre();
        // El número va antes de la extensión: «Nuevo documento (2).txt» sigue
        // abriéndose con el programa de los .txt, «Nuevo documento.txt (2)» no.
        let con_numero = |n: u32| match n {
            1 => format!("{base}{extension}"),
            n => format!("{base} ({n}){extension}"),
        };
        let nombre = (1..)
            .map(con_numero)
            .find(|nombre| !carpeta.join(nombre).exists())
            .expect("la secuencia de números es infinita");
        let ruta = carpeta.join(&nombre);
        let creado = match tipo {
            NuevoElemento::Carpeta => std::fs::create_dir(&ruta),
            // `create_new` y no `File::create`: entre el `exists` y aquí otro
            // proceso puede haber puesto el nombre, y truncarle el archivo
            // sería perder datos de quien sea.
            _ => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&ruta)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(tipo.contenido().as_bytes())
                }),
        };
        if let Err(err) = creado {
            tracing::warn!(nombre, "no se pudo crear: {err}");
            return false;
        }
        if tipo == NuevoElemento::Script {
            use std::os::unix::fs::PermissionsExt;
            if let Err(err) =
                std::fs::set_permissions(&ruta, std::fs::Permissions::from_mode(0o755))
            {
                tracing::warn!(nombre, "no se pudo hacer ejecutable: {err}");
            }
        }
        self.releer(alto_panel, pantalla);
        for elemento in &mut self.elementos {
            elemento.seleccionado = elemento.nombre == nombre;
        }
        true
    }

    /// Manda lo seleccionado a la papelera. `true` si algo cambió.
    ///
    /// A la papelera y no `remove_dir_all`/`remove_file` directos: un
    /// "eliminar" del menú del escritorio se pulsa más fácilmente sin querer
    /// que uno que primero pide confirmar, y sin poder deshacerlo eso es
    /// jugársela con los ficheros de quien sea. `~/.local/share/Trash` es la
    /// misma que usan Dolphin y Nautilus, así que lo que se borra desde aquí
    /// se recupera desde cualquiera de los dos.
    // 0.1.0
    pub fn eliminar_seleccionados(&mut self, alto_panel: f32, pantalla: (f32, f32)) -> bool {
        let rutas: Vec<PathBuf> = self
            .elementos
            .iter()
            .filter(|e| e.seleccionado)
            .map(|e| e.ruta.clone())
            .collect();
        if rutas.is_empty() {
            return false;
        }
        let mut alguno = false;
        for ruta in &rutas {
            match mover_a_papelera(ruta) {
                Ok(()) => alguno = true,
                Err(err) => tracing::warn!(?ruta, "no se pudo mandar a la papelera: {err}"),
            }
        }
        if alguno {
            self.releer(alto_panel, pantalla);
        }
        alguno
    }

    // 0.1.1
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Enseña u oculta los iconos. `true` si cambió algo.
    // 0.1.1
    pub fn mostrar(&mut self, visible: bool, alto_panel: f32, pantalla: (f32, f32)) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        self.arrastre = None;
        self.elementos = if visible { leer() } else { Vec::new() };
        self.recolocar(alto_panel, pantalla);
        true
    }

    /// El color de las carpetas seleccionadas, para marcar su muestra en el
    /// menú. `None` si la selección no es solo de carpetas: un fichero no tiene
    /// color que elegir. `Some(None)` es el automático, o colores mezclados.
    // 0.1.1
    pub fn color_seleccion(&self) -> Option<Option<&str>> {
        let mut seleccion = self.elementos.iter().filter(|e| e.seleccionado).peekable();
        seleccion.peek()?;
        let mut comun: Option<Option<&str>> = None;
        for elemento in seleccion {
            if !elemento.ruta.is_dir() {
                return None;
            }
            let suyo = elemento.color.as_deref();
            comun = match comun {
                None => Some(suyo),
                Some(c) if c == suyo => Some(c),
                Some(_) => Some(None),
            };
        }
        comun
    }

    /// Nombre y color de las carpetas seleccionadas, para el diálogo de color.
    // añadido en la 0.2.0
    pub fn carpetas_seleccionadas(&self) -> Vec<(&str, Option<&str>)> {
        self.elementos
            .iter()
            .filter(|e| e.seleccionado && e.ruta.is_dir())
            .map(|e| (e.nombre.as_str(), e.color.as_deref()))
            .collect()
    }

    /// Pone `color` a las carpetas seleccionadas, o les devuelve el automático
    /// con `None`. `true` si alguna cambió.
    ///
    /// Se escribe en el fichero de bookos-explorer y no en uno propio: una
    /// carpeta es la misma en el escritorio que en el explorador, y dos colores
    /// distintos para ella serían dos verdades.
    // 0.1.1
    pub fn colorear(&mut self, color: Option<&str>) -> bool {
        if color.is_some_and(|c| !hex_valido(c)) {
            tracing::warn!(?color, "color de carpeta no válido");
            return false;
        }
        let rutas: Vec<String> = self
            .elementos
            .iter()
            .filter(|e| e.seleccionado && e.ruta.is_dir())
            .map(|e| e.ruta.to_string_lossy().into_owned())
            .collect();
        if rutas.is_empty() {
            return false;
        }
        if let Err(err) = guardar_colores(&rutas, color) {
            tracing::warn!("no se pudo guardar el color de la carpeta: {err}");
            return false;
        }
        for elemento in self
            .elementos
            .iter_mut()
            .filter(|e| e.seleccionado && e.ruta.is_dir())
        {
            elemento.color = color.map(str::to_string);
            elemento.icono = Some(icono_carpeta(&elemento.nombre, color));
        }
        true
    }

    // 0.1.0
    pub fn cuantos(&self) -> usize {
        self.elementos.len()
    }

    // 0.1.0
    pub fn seleccionado(&self, i: usize) -> bool {
        self.elementos.get(i).is_some_and(|e| e.seleccionado)
    }

    // 0.1.0
    pub fn hay_seleccion(&self) -> bool {
        self.elementos.iter().any(|e| e.seleccionado)
    }

    /// El rectángulo de una celda en lógicos, ya con el desplazamiento del
    /// arrastre si el icono va en él.
    // 0.1.0
    pub fn rect(&self, i: usize) -> Option<(f32, f32, f32, f32)> {
        let elemento = self.elementos.get(i)?;
        let (dx, dy) = match self.arrastre {
            Some(delta) if elemento.seleccionado => delta,
            _ => (0.0, 0.0),
        };
        Some((
            self.zona.0 + elemento.celda.0 as f32 * self.celda().0 + dx,
            self.zona.1 + elemento.celda.1 as f32 * self.celda().1 + dy,
            self.celda().0,
            self.celda().1,
        ))
    }

    /// Qué icono hay en un punto lógico. Se recorre al revés para que, si dos
    /// se solapan durante un arrastre, gane el que se dibuja encima.
    // 0.1.0
    pub fn en(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.elementos.len()).rev().find(|&i| {
            self.rect(i)
                .is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
        })
    }

    /// Selecciona uno, o limpia la selección con `None`. Con `aditivo` (Ctrl)
    /// alterna ese solo y deja el resto como estaba.
    ///
    /// Devuelve `true` si cambió algo, o sea si hay que repintar.
    // 0.1.0
    pub fn seleccionar(&mut self, i: Option<usize>, aditivo: bool) -> bool {
        let mut cambio = false;
        match (i, aditivo) {
            (Some(i), true) => {
                if let Some(elemento) = self.elementos.get_mut(i) {
                    elemento.seleccionado = !elemento.seleccionado;
                    cambio = true;
                }
            }
            (elegido, _) => {
                for (n, elemento) in self.elementos.iter_mut().enumerate() {
                    let quiero = elegido == Some(n);
                    cambio |= elemento.seleccionado != quiero;
                    elemento.seleccionado = quiero;
                }
            }
        }
        cambio
    }

    /// Marca lo que toca la banda elástica. `rect` va en lógicos y ya
    /// normalizado; `previa` es la selección de antes de empezar el gesto, que
    /// se conserva —es lo que hace Ctrl+arrastrar— y va vacía sin Ctrl.
    // 0.1.0
    pub fn banda(&mut self, rect: (f32, f32, f32, f32), previa: &[bool]) -> bool {
        let (bx, by, bw, bh) = rect;
        let celda = self.celda();
        let mut cambio = false;
        for (n, elemento) in self.elementos.iter_mut().enumerate() {
            let ex = self.zona.0 + elemento.celda.0 as f32 * celda.0;
            let ey = self.zona.1 + elemento.celda.1 as f32 * celda.1;
            let toca = bx < ex + celda.0 && bx + bw > ex && by < ey + celda.1 && by + bh > ey;
            // Lo que ya estaba seleccionado antes de empezar la banda se
            // conserva: sin esto, Ctrl+banda deseleccionaría lo anterior.
            let quiero = toca || previa.get(n).copied().unwrap_or(false);
            cambio |= elemento.seleccionado != quiero;
            elemento.seleccionado = quiero;
        }
        cambio
    }

    /// La selección de ahora mismo, para poder sumarle una banda encima.
    // 0.1.0
    pub fn seleccion(&self) -> Vec<bool> {
        self.elementos.iter().map(|e| e.seleccionado).collect()
    }

    /// Corre los iconos seleccionados mientras dura el arrastre.
    // 0.1.0
    pub fn arrastrar(&mut self, delta: (f32, f32)) {
        self.arrastre = Some(delta);
    }

    // 0.1.0
    pub fn arrastrando(&self) -> bool {
        self.arrastre.is_some()
    }

    /// Deja los iconos arrastrados en la celda que les toque y guarda las
    /// posiciones. `true` si alguno cambió de sitio.
    // 0.1.0
    pub fn soltar(&mut self) -> bool {
        let Some((dx, dy)) = self.arrastre.take() else {
            return false;
        };
        let celda = self.celda();
        if !dx.is_finite()
            || !dy.is_finite()
            || self.columnas <= 0
            || self.filas <= 0
            || celda.0 <= 0.0
            || celda.1 <= 0.0
        {
            return false;
        }
        let paso = ((dx / celda.0).round() as i32, (dy / celda.1).round() as i32);
        if paso == (0, 0) {
            return false;
        }
        self.orden = None;
        // Las celdas de los que **no** se mueven son las que hay que respetar;
        // entre los arrastrados se resuelven los choques por orden.
        let mut ocupadas: Vec<(i32, i32)> = self
            .elementos
            .iter()
            .filter(|e| !e.seleccionado)
            .map(|e| e.celda)
            .collect();
        for elemento in self.elementos.iter_mut().filter(|e| e.seleccionado) {
            let destino = (
                (elemento.celda.0 + paso.0).clamp(0, self.columnas - 1),
                (elemento.celda.1 + paso.1).clamp(0, self.filas - 1),
            );
            elemento.celda = if ocupadas.contains(&destino) {
                primer_hueco(&ocupadas, self.columnas, self.filas)
            } else {
                destino
            };
            ocupadas.push(elemento.celda);
        }
        self.guardar_posiciones();
        true
    }

    // 0.1.0
    fn guardar_posiciones(&self) {
        let posiciones: Vec<(&str, (i32, i32))> = self
            .elementos
            .iter()
            .map(|e| (e.nombre.as_str(), e.celda))
            .collect();
        if let Err(err) = crate::config::guardar_escritorio(&posiciones) {
            tracing::warn!("no se pudieron guardar las posiciones del escritorio: {err}");
        }
    }

    /// Qué hacer al abrir un icono: su `Exec` si es un lanzador, y si no,
    /// `xdg-open`, que es quien sabe con qué se abre cada cosa.
    // 0.1.0
    pub fn abrir(&self, i: usize) -> Option<crate::Accion> {
        let elemento = self.elementos.get(i)?;
        Some(crate::Accion::Lanzar(elemento.orden.clone()))
    }

    /// La celda `i` dibujada. Ocupa el buffer entero: es lo que hace que
    /// repintar una selección cueste una celda y no la pantalla.
    // 0.1.0
    pub fn ver(&self, i: usize) -> PanelElement<'_> {
        let Some(elemento) = self.elementos.get(i) else {
            return Space::new().into();
        };
        let celda = self.celda();
        let lado = ICONO * self.tamano.escala();
        let dibujo: PanelElement<'_> = match &elemento.icono {
            Some(ic) => crate::icono::ver(ic, lado, lado),
            None => Space::new().width(Length::Fixed(lado)).into(),
        };
        let dibujo = con_marca(dibujo, lado, elemento.acceso);
        // El nombre va partido en **dos renglones exactos**, hechos aquí y con
        // el ajuste de iced apagado. Dejarle partir a él no vale: no sabe
        // cuándo parar, y «captura de pantalla 2026-08-20.png» salía en tres
        // líneas que se comían la celda de debajo — se vio en la
        // previsualización.
        //
        // El ancho útil son los 96 de la celda menos los 4+4 de su relleno y
        // los 4+4 de la pastilla: con 80 el nombre se salía por los dos lados.
        let ancho_texto = celda.0 - 24.0;
        let nombre = dos_lineas(&elemento.nombre, ancho_texto, TAMANO_NOMBRE);
        // Blanco sobre pastilla oscura, y no los colores del tema: aquí debajo
        // no hay una tarjeta sino el fondo de pantalla, que puede ser
        // cualquier cosa. El texto negro del tema claro sobre un fondo de noche
        // no se lee, y el sistema de diseño no tiene un token para «encima de
        // lo que sea».
        let (pastilla, tinta) = if elemento.seleccionado {
            (tema::acento(), tema::sobre_acento())
        } else {
            (
                Color {
                    a: 0.45,
                    ..Color::BLACK
                },
                Color::WHITE,
            )
        };
        let etiqueta = container(
            text(nombre)
                .size(TAMANO_NOMBRE)
                .color(tinta)
                .align_x(Horizontal::Center)
                .wrapping(iced_core::text::Wrapping::None)
                .shaping(iced_core::text::Shaping::Advanced),
        )
        .padding([1, 4])
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(pastilla.into()),
            border: Border {
                radius: tema::R_CHIP.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let contenido = column![dibujo, etiqueta]
            .spacing(4)
            .align_x(Horizontal::Center);

        container(contenido)
            .width(Length::Fixed(celda.0))
            .height(Length::Fixed(celda.1))
            .padding([6, 4])
            .align_x(Horizontal::Center)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: elemento
                    .seleccionado
                    .then(|| tema::alfa(tema::acento(), 0.22).into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

/// Lo que ocupa una ruta: el archivo, o todo lo de dentro de una carpeta.
///
/// Con tope de 20 000 entradas: esto se calcula al abrir un diálogo, y una
/// carpeta con un `node_modules` dentro tardaría segundos en recorrerse. Por
/// encima del tope la cifra se queda corta, que para «ocupa unos 3 GB» basta.
// añadido en la 0.2.3
pub fn peso(ruta: &Path) -> u64 {
    let mut pendientes = vec![ruta.to_path_buf()];
    let (mut total, mut vistas) = (0u64, 0usize);
    while let Some(r) = pendientes.pop() {
        vistas += 1;
        if vistas > 20_000 {
            break;
        }
        // `symlink_metadata`: un enlace cuenta lo que ocupa él, no lo que
        // apunta (y así un enlace a la carpeta de arriba no da vueltas).
        let Ok(meta) = std::fs::symlink_metadata(&r) else { continue };
        if meta.is_dir() {
            if let Ok(d) = std::fs::read_dir(&r) {
                pendientes.extend(d.filter_map(Result::ok).map(|e| e.path()));
            }
        } else {
            total += meta.len();
        }
    }
    total
}

/// El icono con la marca de acceso directo abajo a la izquierda, como el
/// alias de macOS: un círculo blanco con la flecha. Roto, el icono se apaga y
/// la marca pasa a un aviso naranja.
///
/// Blanco y negro fijos y no del tema: va sobre el fondo de pantalla, igual
/// que la pastilla del nombre (ver [`Escritorio::ver`]).
// añadido en la 0.2.3
fn con_marca(dibujo: PanelElement<'_>, lado: f32, acceso: Acceso) -> PanelElement<'_> {
    let (fondo, glifo) = match acceso {
        Acceso::No => return dibujo,
        Acceso::Si => (Color::WHITE, "enlace-flecha"),
        // El naranja de aviso del HIG, fijo: va sobre el fondo de pantalla.
        Acceso::Roto => (Color::from_rgb8(0xFF, 0x9F, 0x0A), "aviso"),
    };
    // 18 a tamaño medio: legible sin tapar media carpeta.
    let marca = (lado * 0.375).round();
    let glifo = match crate::icono::propio(glifo) {
        Some(ic) => crate::icono::ver_teñido_propio(&ic, marca * 0.6, Color::BLACK),
        None => Space::new().into(),
    };
    iced_widget::stack![
        container(dibujo)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado)),
        container(
            container(glifo)
                .width(Length::Fixed(marca))
                .height(Length::Fixed(marca))
                .center_x(Length::Fixed(marca))
                .center_y(Length::Fixed(marca))
                .style(move |_| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: (marca / 2.0).into(),
                        ..Default::default()
                    },
                    shadow: iced_core::Shadow {
                        color: Color {
                            a: 0.35,
                            ..Color::BLACK
                        },
                        offset: iced_core::Vector::new(0.0, 1.0),
                        blur_radius: 3.0,
                    },
                    ..Default::default()
                })
        )
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .align_left(Length::Fixed(lado))
        .align_bottom(Length::Fixed(lado)),
    ]
    .into()
}

/// Parte un nombre en como mucho dos renglones que quepan en `ancho`.
///
/// El sobrante se remata con puntos suspensivos. Se hace aquí y no con el
/// ajuste de línea de iced porque iced no sabe parar a las dos líneas.
// 0.1.0
pub(crate) fn dos_lineas(nombre: &str, ancho: f32, tamaño: f32) -> String {
    let (primera, resto) = cortar(nombre, ancho, tamaño);
    if resto.is_empty() {
        return primera;
    }
    let (segunda, sobra) = cortar(&resto, ancho, tamaño);
    if sobra.is_empty() {
        return format!("{primera}\n{segunda}");
    }
    // Los puntos ocupan sitio: la segunda línea se corta dejándoselo.
    let hueco = ancho - crate::widget::ancho_de("…", tamaño);
    let (segunda, _) = cortar(&resto, hueco.max(0.0), tamaño);
    format!("{primera}\n{}…", segunda.trim_end())
}

/// Parte `texto` en renglones que quepan en `ancho`, respetando los saltos de
/// línea, hasta `max`. Lo que sobra se remata con puntos suspensivos en el
/// último renglón, como [`dos_lineas`] pero para cualquier número de líneas.
// 0.1.0
pub(crate) fn lineas(texto: &str, ancho: f32, tamaño: f32, max: usize) -> Vec<String> {
    let mut salida: Vec<String> = Vec::new();
    for parrafo in texto.lines() {
        let mut resto = parrafo.to_string();
        loop {
            if salida.len() == max {
                // Ya no caben más: el último renglón avisa de que hay más.
                if let Some(ultima) = salida.last_mut() {
                    let hueco = ancho - crate::widget::ancho_de("…", tamaño);
                    let (recorte, _) = cortar(ultima, hueco.max(0.0), tamaño);
                    *ultima = format!("{}…", recorte.trim_end());
                }
                return salida;
            }
            let (linea, sobra) = cortar(&resto, ancho, tamaño);
            salida.push(linea);
            if sobra.is_empty() {
                break;
            }
            resto = sobra;
        }
    }
    salida
}

/// Corta `texto` por donde quepa en `ancho`: devuelve el trozo y lo que queda.
///
/// La primera tijera se pone con el ancho medio por letra y se corrige midiendo
/// de verdad. Medir carácter a carácter sería un `Paragraph` por letra —treinta
/// por nombre— y esto se paga en cada repintado de celda, o sea en cada celda
/// que cruza la banda elástica.
// 0.1.0
fn cortar(texto: &str, ancho: f32, tamaño: f32) -> (String, String) {
    if crate::widget::ancho_de(texto, tamaño) <= ancho {
        return (texto.to_string(), String::new());
    }
    let letras: Vec<char> = texto.chars().collect();
    let medio = crate::widget::ancho_de(texto, tamaño) / letras.len() as f32;
    let mut caben = (ancho / medio).floor().max(1.0) as usize;
    caben = caben.min(letras.len());
    while caben > 1 {
        let linea: String = letras[..caben].iter().collect();
        if crate::widget::ancho_de(&linea, tamaño) <= ancho {
            break;
        }
        caben -= 1;
    }
    // Se prefiere partir por un separador, si cae en la segunda mitad de lo que
    // cabe: cortar «informe-trimestral.pdf» por en medio de «trimes|tral» se
    // lee peor que dejar el guion al final del renglón.
    let corte = letras[..caben]
        .iter()
        .rposition(|c| matches!(c, ' ' | '-' | '_'))
        .map(|i| i + 1)
        .filter(|i| i * 2 >= caben)
        .unwrap_or(caben);
    (
        letras[..corte]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string(),
        letras[corte..].iter().collect(),
    )
}

/// El primer hueco libre llenando por columnas, de arriba abajo.
///
/// Si no queda ninguno se apila en la última: perder un icono porque la
/// pantalla es pequeña es peor que solaparlo.
// 0.1.0
fn primer_hueco(ocupadas: &[(i32, i32)], columnas: i32, filas: i32) -> (i32, i32) {
    (0..columnas)
        .flat_map(|col| (0..filas).map(move |fila| (col, fila)))
        .find(|celda| !ocupadas.contains(celda))
        .unwrap_or((columnas - 1, filas - 1))
}

/// Lo que hay en la carpeta Escritorio: carpetas primero y luego ficheros, cada
/// grupo por nombre.
///
/// Se ignoran los ocultos, como cualquier gestor de archivos, y los que no se
/// pueden leer: un `.desktop` roto es un icono menos, no un escritorio sin
/// iconos.
// 0.1.0
// modified 0.1.1 -- lee los colores de carpeta del explorador
fn leer() -> Vec<Elemento> {
    let Some(carpeta) = carpeta_escritorio() else {
        return Vec::new();
    };
    let Ok(entradas) = std::fs::read_dir(&carpeta) else {
        return Vec::new();
    };
    let colores = colores_explorer();
    let mut elementos: Vec<Elemento> = entradas
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Elemento::leer(&e.path(), &colores))
        .collect();
    elementos.sort_by(|a, b| {
        let carpeta_a = a.ruta.is_dir();
        let carpeta_b = b.ruta.is_dir();
        carpeta_b
            .cmp(&carpeta_a)
            .then_with(|| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()))
    });
    elementos
}

impl Elemento {
    // 0.1.0
    // modified 0.1.1 -- carpetas con su color y lanzadores sin icono con el de fichero
    fn leer(ruta: &Path, colores: &HashMap<String, String>) -> Option<Self> {
        let nombre_fichero = ruta.file_name()?.to_string_lossy().into_owned();
        // `symlink_metadata` no sigue el enlace: es la única forma de saber
        // que lo es. Si sigue roto, `exists` (que sí lo sigue) dice que no.
        let acceso = match std::fs::symlink_metadata(ruta).map(|m| m.file_type().is_symlink()) {
            Ok(true) if ruta.exists() => Acceso::Si,
            Ok(true) => Acceso::Roto,
            _ => Acceso::No,
        };
        // Un `.desktop` en el escritorio es un lanzador, no un fichero de
        // texto: enseña su `Name` y su icono, y al abrirlo se ejecuta su
        // `Exec`. Es lo único que distingue un escritorio de un listado.
        if ruta.extension().is_some_and(|e| e == "desktop")
            && let Some(app) = crate::apps::leer_lanzador(ruta)
        {
            return Some(Self {
                nombre: app.nombre,
                ruta: ruta.to_path_buf(),
                orden: app.exec,
                // Un lanzador cuyo icono no está en ningún tema no puede quedar
                // en blanco: se le pone el de fichero, como a cualquier otro.
                icono: crate::icono::cargar(&app.icono).or_else(|| Some(icono_fichero(ruta))),
                color: None,
                acceso,
                celda: (-1, -1),
                seleccionado: false,
            });
        }
        let carpeta = ruta.is_dir();
        let color = carpeta
            .then(|| colores.get(ruta.to_string_lossy().as_ref()).cloned())
            .flatten()
            .filter(|c| hex_valido(c));
        // Un acceso directo roto no tiene a qué parecerse: una hoja gris
        // sin extensión dice «aquí había algo» sin fingir lo que era.
        let icono = match (acceso, carpeta) {
            (Acceso::Roto, _) => icono_fichero(Path::new("")),
            (_, true) => icono_carpeta(&nombre_fichero, color.as_deref()),
            (_, false) => icono_fichero(ruta),
        };
        Some(Self {
            nombre: nombre_fichero,
            ruta: ruta.to_path_buf(),
            orden: format!("xdg-open {}", entrecomillar(ruta)),
            icono: Some(icono),
            color,
            acceso,
            celda: (-1, -1),
            seleccionado: false,
        })
    }
}

// --- Iconos de carpeta y fichero ---------------------------------------------
//
// Los mismos que dibuja bookos-explorer (su `src/icons.js`, el «Sistema A»):
// relleno sólido de un color por función y nada de tema de iconos. Con los del
// tema —Breeze, aquí— una carpeta del escritorio y la misma carpeta abierta en
// el explorador parecían de dos sistemas distintos, y además dependían de que
// el tema tuviera `places/folder`. Los trazados y los tonos se copian tal cual:
// si cambian allí, hay que cambiarlos aquí.

/// Las muestras del selector de color, en el orden del explorador.
pub const MUESTRAS: [&str; 6] = [
    "#FFCC00", "#FF3B30", "#FF9500", "#34C759", "#007AFF", "#AF52DE",
];

const CARPETA_POR_DEFECTO: &str = "#3B9BFF";

/// El color que le toca a una carpeta por su nombre, si no tiene uno puesto.
// 0.1.1
fn color_automatico(nombre: &str) -> &'static str {
    match nombre.to_lowercase().as_str() {
        "downloads" | "descargas" => "#AF52DE",
        "documents" | "documentos" => "#FF3B30",
        "music" | "música" | "musica" => "#34C759",
        "pictures" | "images" | "imágenes" | "imagenes" => "#FF9500",
        "videos" | "vídeos" => "#5856D6",
        "desktop" | "escritorio" => "#00C7BE",
        "public" | "público" | "publico" => "#FFCC00",
        "templates" | "plantillas" => "#FF2D55",
        _ => CARPETA_POR_DEFECTO,
    }
}

/// La pestaña se aclara con opacidad y no con un segundo tono, como en el
/// explorador: así un color nuevo es una sola cadena.
// 0.1.1
pub(crate) fn icono_carpeta(nombre: &str, color: Option<&str>) -> Icono {
    let c = color.unwrap_or_else(|| color_automatico(nombre));
    crate::icono::desde_svg(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 82"><path d="M2 16a12 12 0 0 1 12-12h20.7a12 12 0 0 1 8.5 3.5L47 12h39a12 12 0 0 1 12 12v8H2z" fill="{c}" opacity=".55"/><rect x="2" y="20" width="96" height="60" rx="13" fill="{c}"/></svg>"#
    ))
}

/// La hoja con la esquina doblada, del color de su tipo y con la extensión
/// escrita encima.
// 0.1.1
fn icono_fichero(ruta: &Path) -> Icono {
    let ext = ruta
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let color = color_de_tipo(&ext);
    // La etiqueta acaba dentro del SVG: los caracteres que lo romperían fuera.
    let etiqueta: String = ext
        .chars()
        .filter(|c| !matches!(c, '&' | '<' | '>'))
        .take(4)
        .collect::<String>()
        .to_uppercase();
    // Familias con nombre antes de `sans-serif`: la base de fuentes de resvg
    // resuelve el genérico a «Arial», que en Fedora no está, y con solo
    // `sans-serif` la etiqueta no salía —se vio en la previsualización—.
    let texto = match etiqueta.is_empty() {
        true => String::new(),
        false => format!(
            r##"<text x="50" y="60" text-anchor="middle" font-size="21" font-weight="700" fill="#fff" font-family="Inter, Noto Sans, Cantarell, DejaVu Sans, sans-serif">{etiqueta}</text>"##
        ),
    };
    crate::icono::desde_svg(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 82"><path d="M18 9a8 8 0 0 1 8-8h30l22 22v51a8 8 0 0 1-8 8H26a8 8 0 0 1-8-8z" fill="{color}"/><path d="M56 1l22 22H60a4 4 0 0 1-4-4z" fill="#fff" opacity=".45"/>{texto}</svg>"##
    ))
}

/// El color de cada tipo de fichero, por extensión: la tabla `KINDS` del
/// explorador. Lo que no se reconoce va en gris.
// 0.1.1
fn color_de_tipo(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "avif" | "heic" | "ico"
        | "tiff" | "tif" | "xcf" | "psd" | "kra" | "ora" | "odg" | "fodg" => "#FF9500",
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" => {
            "#5856D6"
        }
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" => "#34C759",
        "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" | "zst" | "deb" | "rpm" | "pkg"
        | "iso" => "#FF2D55",
        "pdf" => "#FF3B30",
        "doc" | "docx" | "docm" | "dotx" | "odt" | "ott" | "fodt" | "rtf" | "txt" | "md"
        | "epub" => "#007AFF",
        "xls" | "xlsx" | "xlsm" | "ods" | "ots" | "fods" | "csv" | "tsv" => "#00C7BE",
        "ppt" | "pptx" | "pps" | "ppsx" | "odp" | "otp" | "fodp" | "key" => "#AF52DE",
        "js" | "ts" | "jsx" | "tsx" | "rs" | "py" | "c" | "h" | "cpp" | "go" | "java" | "sh"
        | "json" | "toml" | "yaml" | "yml" | "html" | "css" | "xml" | "sql" | "lua" | "rb"
        | "php" => "#5AC8FA",
        _ => "#8E8E93",
    }
}

/// Solo `#RRGGBB`, como exige el explorador: el valor acaba dentro de un
/// atributo `fill`, y cualquier otra cosa sería inyectar en el SVG.
// 0.1.1
fn hex_valido(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// `~/.config/bookos-explorer/colors.json`: ruta absoluta → color.
// 0.1.1
fn ruta_colores() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("bookos-explorer/colors.json"))
}

/// Un fichero que no está o no se entiende es un mapa vacío: todas las
/// carpetas en su color automático, que es lo que enseña el explorador igual.
// 0.1.1
fn colores_explorer() -> HashMap<String, String> {
    ruta_colores()
        .and_then(|ruta| std::fs::read_to_string(ruta).ok())
        .and_then(|texto| serde_json::from_str(&texto).ok())
        .unwrap_or_default()
}

/// Lee, cambia esas rutas y reescribe. Se relee justo antes para no pisar lo
/// que haya cambiado el explorador desde que se cargó el escritorio.
// 0.1.1
fn guardar_colores(rutas: &[String], color: Option<&str>) -> std::io::Result<()> {
    let ruta = ruta_colores().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "sin HOME ni XDG_CONFIG_HOME")
    })?;
    let mut mapa = colores_explorer();
    for carpeta in rutas {
        match color {
            Some(c) => mapa.insert(carpeta.clone(), c.to_string()),
            None => mapa.remove(carpeta),
        };
    }
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let texto = serde_json::to_string(&mapa).map_err(std::io::Error::other)?;
    std::fs::write(ruta, texto)
}

/// Entrecomilla una ruta para `sh -c`: es lo que ejecuta el compositor, y un
/// nombre con espacios o con `;` no puede acabar siendo dos órdenes.
// 0.1.0
pub fn entrecomillar(ruta: &Path) -> String {
    format!("'{}'", ruta.to_string_lossy().replace('\'', r"'\''"))
}

/// Dónde está la carpeta del escritorio.
///
/// Primero lo que diga `user-dirs.dirs`, que es donde lo escriben `xdg-user-dirs`
/// y los entornos de escritorio; si no está, los dos nombres de siempre. Sin
/// esto, en una sesión en castellano se leería `~/Desktop` —que no existe— y el
/// escritorio saldría vacío.
/// Manda un fichero o carpeta a la papelera de freedesktop
/// (`~/.local/share/Trash`), la que siguen Dolphin, Nautilus y el resto de
/// gestores que respetan la especificación.
// 0.1.0
fn mover_a_papelera(ruta: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let datos = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "sin HOME ni XDG_DATA_HOME"))?;
    let papelera = datos.join("Trash");
    let ficheros = papelera.join("files");
    let info = papelera.join("info");
    std::fs::create_dir_all(&ficheros)?;
    std::fs::create_dir_all(&info)?;

    let nombre = ruta
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "ruta sin nombre de fichero"))?
        .to_string_lossy()
        .into_owned();
    // Nombre libre dentro de la papelera: puede que ya haya algo con el mismo
    // nombre —de otra carpeta, o de un borrado anterior— y sobrescribirlo
    // perdería lo que ya estaba ahí.
    let (base, extension) = match nombre.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b.to_string(), format!(".{e}")),
        _ => (nombre.clone(), String::new()),
    };
    let origen_absoluto = if ruta.is_absolute() {
        ruta.to_path_buf()
    } else {
        std::env::current_dir()?.join(ruta)
    };
    let contenido = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        codificar_ruta(&origen_absoluto),
        fecha_papelera(),
    );

    // El .trashinfo va primero y con `create_new`, como pide la
    // especificación: crearlo en exclusiva es lo que reserva el nombre, y si
    // se escribiera después del `rename` y fallara, el fichero quedaría en
    // Trash/files sin nada que diga de dónde vino.
    let mut destino_nombre = nombre.clone();
    let mut intento = 2;
    let ficha = loop {
        let ficha = info.join(format!("{destino_nombre}.trashinfo"));
        if !ficheros.join(&destino_nombre).exists() {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&ficha)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    if let Err(e) = f.write_all(contenido.as_bytes()) {
                        let _ = std::fs::remove_file(&ficha);
                        return Err(e);
                    }
                    break ficha;
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        destino_nombre = format!("{base} {intento}{extension}");
        intento += 1;
    };
    if let Err(e) = std::fs::rename(ruta, ficheros.join(&destino_nombre)) {
        let _ = std::fs::remove_file(&ficha);
        return Err(e);
    }
    Ok(())
}

/// Codifica una ruta como pide la especificación de la papelera: los bytes
/// fuera del juego "sin reservar" de RFC 3986, en `%XX`.
///
/// A mano y no con una dependencia: una ruta de escritorio real lleva letras,
/// números y como mucho espacios o acentos, y son unos pocos bytes por
/// fichero borrado.
// 0.1.0
fn codificar_ruta(ruta: &Path) -> String {
    let mut salida = String::new();
    for byte in ruta.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                salida.push(byte as char);
            }
            _ => {
                let _ = write!(salida, "%{byte:02X}");
            }
        }
    }
    salida
}

/// `DeletionDate` en ISO 8601, tal como pide la especificación de la
/// papelera.
///
/// En UTC y no en hora local: este crate no lleva ninguna dependencia de
/// zonas horarias, y la especificación pide "una fecha", sin exigir el
/// desplazamiento local. Dolphin y Nautilus la leen igual.
// 0.1.0
fn fecha_papelera() -> String {
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let segundos = ahora.as_secs() as i64;
    let dias = segundos.div_euclid(86_400);
    let resto = segundos.rem_euclid(86_400);
    let (h, m, s) = (resto / 3600, (resto / 60) % 60, resto % 60);
    let (y, mes, d) = civil_desde_dias(dias);
    format!("{y:04}-{mes:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

/// De días desde 1970-01-01 a año/mes/día del calendario gregoriano.
///
/// El algoritmo de Howard Hinnant, de dominio público: aritmética entera
/// pura, sin tabla de meses ni de años bisiestos escrita a mano. Comprobado
/// contra `datetime` de Python en varias fechas, incluyendo negativas y
/// bisiestos.
// 0.1.0
fn civil_desde_dias(dias: i64) -> (i64, u32, u32) {
    let z = dias + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mes <= 2 { y + 1 } else { y };
    (y, mes, d)
}

// 0.1.0
pub fn carpeta_escritorio() -> Option<PathBuf> {
    // La escotilla es para la previsualización y para las pruebas: apuntar a
    // una carpeta de mentira es la única forma de mirar la rejilla sin
    // ensuciar el escritorio de verdad de quien ejecuta el ejemplo.
    if let Some(dir) = std::env::var_os("BOOKOS_ESCRITORIO") {
        return Some(PathBuf::from(dir));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if let Some(dir) = leer_user_dirs(&home)
        && dir.is_dir()
    {
        return Some(dir);
    }
    [home.join("Escritorio"), home.join("Desktop")]
        .into_iter()
        .find(|d| d.is_dir())
}

// 0.1.0
fn leer_user_dirs(home: &Path) -> Option<PathBuf> {
    let ruta = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir).join("user-dirs.dirs"),
        None => home.join(".config/user-dirs.dirs"),
    };
    let texto = std::fs::read_to_string(ruta).ok()?;
    let valor = texto
        .lines()
        .map(str::trim)
        .find_map(|linea| linea.strip_prefix("XDG_DESKTOP_DIR="))?
        .trim_matches('"');
    // El fichero escribe la ruta como `"$HOME/Escritorio"`.
    Some(match valor.strip_prefix("$HOME/") {
        Some(resto) => home.join(resto),
        None => PathBuf::from(valor),
    })
}

/// Las posiciones que el usuario dejó puestas arrastrando.
// 0.1.0
fn posiciones_guardadas() -> Vec<(String, (i32, i32))> {
    crate::config::cargar_escritorio()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escritorio(cuantos: usize, pantalla: (f32, f32)) -> Escritorio {
        let mut e = Escritorio {
            elementos: (0..cuantos)
                .map(|n| Elemento {
                    nombre: format!("fichero {n}"),
                    ruta: PathBuf::from(format!("/tmp/fichero {n}")),
                    orden: String::new(),
                    icono: None,
                    color: None,
                    acceso: Acceso::No,
                    celda: (-1, -1),
                    seleccionado: false,
                })
                .collect(),
            tamano: Tamano::Mediano,
            orden: None,
            zona: (0.0, 0.0, 0.0, 0.0),
            filas: 1,
            columnas: 1,
            arrastre: None,
            visible: true,
        };
        e.zona = (
            MARGEN_X,
            32.0 + MARGEN_Y,
            pantalla.0 - MARGEN_X * 2.0,
            pantalla.1 - 32.0 - MARGEN_Y - RESERVA_DOCK,
        );
        e.columnas = (e.zona.2 / CELDA.0).floor().max(1.0) as i32;
        e.filas = (e.zona.3 / CELDA.1).floor().max(1.0) as i32;
        // Sin leer del disco: `asignar_celdas` consultaría las posiciones
        // guardadas del usuario que ejecute los tests.
        let mut ocupadas = Vec::new();
        for elemento in &mut e.elementos {
            elemento.celda = primer_hueco(&ocupadas, e.columnas, e.filas);
            ocupadas.push(elemento.celda);
        }
        e
    }

    /// La rejilla se llena por columnas, de arriba abajo: el segundo icono va
    /// **debajo** del primero, no a su derecha.
    #[test]
    fn la_rejilla_se_llena_por_columnas() {
        let e = escritorio(3, (1440.0, 900.0));
        let (x0, y0, _, _) = e.rect(0).expect("hay tres iconos");
        let (x1, y1, _, _) = e.rect(1).expect("hay tres iconos");
        assert_eq!(x0, x1);
        assert_eq!(y1 - y0, CELDA.1);
    }

    /// La banda elástica marca lo que toca y desmarca lo que deja de tocar.
    #[test]
    fn la_banda_marca_lo_que_cruza() {
        let mut e = escritorio(3, (1440.0, 900.0));
        let (x, y, w, h) = e.rect(1).expect("hay tres iconos");
        let previa = vec![false; 3];
        assert!(e.banda((x + 2.0, y + 2.0, w - 4.0, h - 4.0), &previa));
        assert!(!e.seleccionado(0) && e.seleccionado(1) && !e.seleccionado(2));
        // Una banda que ya no toca nada deja la selección vacía.
        assert!(e.banda((0.0, 0.0, 1.0, 1.0), &previa));
        assert!(!e.hay_seleccion());
    }

    /// Arrastrar mueve la selección de celda, y lo que estaba en el destino no
    /// se pisa: el que llega busca hueco.
    #[test]
    fn al_soltar_el_icono_cae_en_una_celda_libre() {
        let mut e = escritorio(2, (1440.0, 900.0));
        e.seleccionar(Some(1), false);
        // Justo una celda hacia arriba, o sea encima del icono 0.
        e.arrastrar((0.0, -CELDA.1));
        assert!(e.soltar());
        assert_ne!(e.elementos[0].celda, e.elementos[1].celda);
    }

    /// Las muestras solo salen con carpetas seleccionadas, y marcan el color
    /// solo si todas comparten el mismo.
    #[test]
    fn el_color_de_la_seleccion_es_el_comun_de_sus_carpetas() {
        let mut e = escritorio(3, (1440.0, 900.0));
        // `/tmp` es una carpeta en cualquier máquina; `/tmp/fichero n` no existe
        // y cuenta como fichero.
        e.elementos[0].ruta = PathBuf::from("/tmp");
        e.elementos[1].ruta = PathBuf::from("/tmp");
        assert_eq!(e.color_seleccion(), None, "sin selección no hay muestras");
        e.seleccionar(Some(0), false);
        e.seleccionar(Some(1), true);
        e.elementos[0].color = Some("#FF3B30".into());
        e.elementos[1].color = Some("#FF3B30".into());
        assert_eq!(e.color_seleccion(), Some(Some("#FF3B30")));
        e.elementos[1].color = None;
        assert_eq!(
            e.color_seleccion(),
            Some(None),
            "mezclados no marcan ninguna"
        );
        e.seleccionar(Some(2), true);
        assert_eq!(e.color_seleccion(), None, "un fichero no tiene color");
    }

    /// Un nombre con comillas o espacios no puede convertirse en dos órdenes al
    /// pasar por `sh -c`.
    #[test]
    fn la_ruta_va_entrecomillada() {
        let orden = entrecomillar(Path::new("/tmp/a b'; rm -rf ~"));
        assert_eq!(orden, r"'/tmp/a b'\''; rm -rf ~'");
    }

    #[test]
    fn civil_desde_dias_coincide_con_el_calendario() {
        // (días desde 1970-01-01, año, mes, día). Incluye negativos —antes de
        // la época— y un bisiesto, que es donde este tipo de aritmética suele
        // fallar.
        for (dias, y, m, d) in [
            (0i64, 1970i64, 1u32, 1u32),
            (1, 1970, 1, 2),
            (365, 1971, 1, 1),
            (366, 1971, 1, 2),
            (19_000, 2022, 1, 8),
            (20_000, 2024, 10, 4),
            (-1, 1969, 12, 31),
            (-365, 1969, 1, 1),
        ] {
            assert_eq!(civil_desde_dias(dias), (y, m, d), "días={dias}");
        }
    }

    #[test]
    fn codificar_ruta_deja_intactos_los_caracteres_normales_y_escapa_el_resto() {
        assert_eq!(
            codificar_ruta(Path::new("/home/eve/Escritorio/informe.pdf")),
            "/home/eve/Escritorio/informe.pdf"
        );
        assert_eq!(
            codificar_ruta(Path::new("/home/eve/Mi carpeta (2)")),
            "/home/eve/Mi%20carpeta%20%282%29"
        );
    }

    /// Crear y eliminar de verdad, sobre un directorio temporal: es la única
    /// forma de comprobar que la carpeta aparece donde toca y que lo borrado
    /// llega a la papelera, y no solo que la aritmética de nombres es
    /// correcta.
    #[test]
    fn crear_y_eliminar_una_carpeta_de_verdad() {
        let carpeta = crate::emergente::t("Nueva carpeta", "New folder");
        let sufijo = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        );
        let escritorio_tmp = std::env::temp_dir().join(format!("bookos-test-escritorio-{sufijo}"));
        let datos_tmp = std::env::temp_dir().join(format!("bookos-test-datos-{sufijo}"));
        std::fs::create_dir_all(&escritorio_tmp).expect("crear el escritorio de prueba");
        // SAFETY: `BOOKOS_ESCRITORIO` es la escotilla que existe justo para
        // esto, y ningún otro test de este binario lee `XDG_DATA_HOME`: los
        // dos son seguros de tocar aquí sin pisarle una variable a otro test.
        unsafe {
            std::env::set_var("BOOKOS_ESCRITORIO", &escritorio_tmp);
            std::env::set_var("XDG_DATA_HOME", &datos_tmp);
        }

        let mut e = Escritorio::new(32.0, (1440.0, 900.0), true, Tamano::Mediano);
        assert_eq!(e.cuantos(), 0, "el directorio de prueba nace vacío");

        assert!(e.crear_nuevo(NuevoElemento::Carpeta, 32.0, (1440.0, 900.0)));
        assert_eq!(e.cuantos(), 1);
        assert!(escritorio_tmp.join(carpeta).is_dir());
        assert!(
            e.elementos[0].seleccionado,
            "la carpeta recién creada queda seleccionada"
        );

        // Un archivo lleva el número antes de la extensión, y un script nace ejecutable.
        let texto = crate::emergente::t("Nuevo documento de texto", "New text document");
        let script = crate::emergente::t("Nuevo script", "New script");
        assert!(e.crear_nuevo(NuevoElemento::Texto, 32.0, (1440.0, 900.0)));
        assert!(e.crear_nuevo(NuevoElemento::Texto, 32.0, (1440.0, 900.0)));
        assert!(escritorio_tmp.join(format!("{} (2).txt", texto)).is_file());
        assert!(e.crear_nuevo(NuevoElemento::Script, 32.0, (1440.0, 900.0)));
        {
            use std::os::unix::fs::PermissionsExt;
            let modo = std::fs::metadata(escritorio_tmp.join(format!("{script}.sh")))
                .expect("el script se acaba de crear")
                .permissions()
                .mode();
            assert_eq!(modo & 0o111, 0o111);
        }
        std::fs::remove_file(escritorio_tmp.join(format!("{texto}.txt"))).unwrap();
        std::fs::remove_file(escritorio_tmp.join(format!("{texto} (2).txt"))).unwrap();
        std::fs::remove_file(escritorio_tmp.join(format!("{script}.sh"))).unwrap();
        e.releer(32.0, (1440.0, 900.0));
        assert_eq!(e.cuantos(), 1);

        // Una segunda no choca con el nombre de la primera.
        assert!(e.crear_nuevo(NuevoElemento::Carpeta, 32.0, (1440.0, 900.0)));
        assert_eq!(e.cuantos(), 2);
        assert!(escritorio_tmp.join(format!("{carpeta} (2)")).is_dir());

        for elemento in &mut e.elementos {
            elemento.seleccionado = elemento.nombre == carpeta;
        }
        assert!(e.eliminar_seleccionados(32.0, (1440.0, 900.0)));
        assert_eq!(e.cuantos(), 1, "la eliminada ya no está en la lista");
        assert!(
            !escritorio_tmp.join(carpeta).exists(),
            "salió de donde estaba"
        );
        assert!(
            datos_tmp.join(format!("Trash/files/{carpeta}")).is_dir(),
            "llegó a la papelera, no se borró de verdad"
        );
        let info =
            std::fs::read_to_string(datos_tmp.join(format!("Trash/info/{carpeta}.trashinfo")))
                .expect("hay .trashinfo");
        assert!(info.starts_with("[Trash Info]\n"));
        assert!(info.contains("Path="));
        assert!(info.contains("DeletionDate="));

        let _ = std::fs::remove_dir_all(&escritorio_tmp);
        let _ = std::fs::remove_dir_all(&datos_tmp);
        // SAFETY: mismo argumento que al ponerlas.
        unsafe {
            std::env::remove_var("BOOKOS_ESCRITORIO");
            std::env::remove_var("XDG_DATA_HOME");
        }
    }

    #[test]
    fn soltar_con_rejilla_vacia_o_delta_no_finito_no_entra_en_panico() {
        let mut e = Escritorio::new(32.0, (0.0, 0.0), false, Tamano::Mediano);
        e.columnas = 0;
        e.filas = 0;
        e.arrastrar((100.0, 100.0));
        assert!(!e.soltar());
        assert!(!e.arrastrando());

        let mut valid = Escritorio::new(32.0, (1920.0, 1080.0), false, Tamano::Mediano);
        valid.arrastrar((f32::NAN, 40.0));
        assert!(!valid.soltar());
    }
}
