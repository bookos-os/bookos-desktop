//! El cursor del ratón.
//!
//! Un compositor Wayland dibuja el cursor él mismo: el kernel no pinta nada y
//! el cliente solo *propone* una imagen. Sin esto el puntero existe —los
//! clientes reciben `enter`, `motion` y `button`— pero es invisible, que desde
//! fuera se parece mucho a que la entrada no funcione.
//!
//! Hay dos casos:
//!
//! - **El cliente propone su propia superficie** (`CursorImageStatus::Surface`),
//!   típico de una terminal con la barra en I o de un editor. Se dibuja esa
//!   superficie tal cual, respetando su hotspot.
//! - **El cliente no propone nada** y toca el cursor del tema. Se carga del
//!   tema XCursor del sistema, que es el que ya usa el resto del escritorio.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{
    CropRenderElement, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Transform};

smithay::render_elements! {
    /// Lo que el compositor superpone a las ventanas: shell y cursor.
    ///
    /// Hacen falta dos tipos porque el cursor puede ser una imagen nuestra
    /// (`Memory`) o una superficie del propio cliente (`Surface`) — un editor
    /// pone su barra en I, y si solo supiéramos dibujar la nuestra, el cursor
    /// desaparecería justo encima del texto.
    /// Va fijado a GLES y no genérico sobre el renderer: los dos backends
    /// componen con GLES, y el cristal necesita copiar el framebuffer, que es
    /// GL crudo y no existe en un renderer cualquiera.
    pub OverlayElement<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    /// La ventana congelada y deformada del minimizar «magic lamp». Es la
    /// única que dibuja con un shader propio sobre una textura nuestra.
    Genio=crate::genio::Elemento,
    /// Una textura nuestra con alfa: las ventanas que se están cerrando
    /// —[`crate::cierre`]— y la foto del tema anterior al cambiar de claro a
    /// oscuro —[`crate::fundido`]—.
    Textura=smithay::backend::renderer::element::texture::TextureRenderElement<
        smithay::backend::renderer::gles::GlesTexture,
    >,
    /// Shaders de píxeles: velo de captura, bandas y puntos del salvapantallas.
    Velo=smithay::backend::renderer::gles::element::PixelShaderElement,
    /// Un rectángulo de color, para el velo del launchpad. Va aparte porque
    /// pintarlo dentro del buffer del shell obligaría a rasterizar la pantalla
    /// entera en CPU; aquí lo compone la GPU y no cuesta nada.
    Color=SolidColorRenderElement,
    /// Una superficie a otro tamaño: la entrada de ventanas y la compensación
    /// del cursor X11. Una ventana quieta va por `Surface` para conservar su
    /// geometría exacta en el seguimiento de daños.
    Escalada=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    /// La barra de título mientras su ventana crece al aparecer. Va aparte de
    /// `Memory` por lo mismo que `Escalada`: una barra quieta no pasa por la
    /// capa de escala, para no darle al damage tracker geometría redondeada.
    MemoriaEscalada=RescaleRenderElement<MemoryRenderBufferRenderElement<GlesRenderer>>,
    /// Superficie viva ajustada al hueco de Meta+Tab.
    Miniatura=CropRenderElement<RelocateRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>>,
    /// La misma, recortada con esquinas redondeadas: las ventanas de la vista
    /// de escritorios. Ver [`crate::esquinas`].
    Redondeada=crate::esquinas::Redondeada,
    /// El fondo esmerilado que va detrás del panel y del dock. Solo existe
    /// para el renderer de GLES: desenfocar necesita copiar el framebuffer, y
    /// eso es GL crudo.
    Cristal=crate::desenfoque::Desenfoque,
}

/// El tema de cursores de BookOS.
///
/// El nombre es el del **directorio** en `~/.local/share/icons`, no el `Name=`
/// de su `index.theme`: es lo que busca xcursor. Su `Inherits=breeze_cursors`
/// cubre los cursores raros que BookOS no dibuja.
const TEMA: &str = "BookOS-Dark";

// 0.1.1
pub fn nombre_tema() -> String {
    std::env::var("XCURSOR_THEME").unwrap_or_else(|_| TEMA.to_string())
}

/// Un cursor ya cargado y listo para subir a textura.
struct Imagen {
    buffer: MemoryRenderBuffer,
    /// Tamaño con el que hay que dibujarlo, en píxeles **lógicos**.
    ///
    /// Lógicos y no los del fichero: `from_buffer` toma el tamaño en lógicos y
    /// lo multiplica él por la escala de la pantalla. Pasarle los píxeles del
    /// fichero dibujaba el cursor `escala` veces más grande — a 1,75 se veía al
    /// doble, porque además el más cercano a 24×1,75 = 42 es el nominal de 48.
    /// Sale de [`a_logico`], que va por el nominal del fichero.
    logico: (i32, i32),
    /// Tamaño real de la imagen del fichero, en píxeles.
    pixeles: (i32, i32),
    /// Tamaño nominal declarado en el fichero XCursor, para escalar el hotspot
    /// cuando se carga una variante de mayor resolución.
    del_fichero: u32,
    /// Punto "activo" dentro de la imagen: la punta de la flecha, en píxeles
    /// del fichero.
    hotspot: (i32, i32),
}

pub struct CursorTheme {
    nombre: String,
    /// Tamaño lógico pedido, que es también el que se les dice a los clientes.
    nominal: u32,
    /// Tamaño en píxeles físicos al que se pide cada imagen.
    objetivo: u32,
    /// Cursores ya buscados. `None` recuerda los que no están, para no volver a
    /// recorrer el disco en cada fotograma preguntando por el mismo que falta.
    cache: RefCell<HashMap<String, Option<Rc<Imagen>>>>,
    /// Caché a alta resolución para cuando el cursor se agranda al agitar.
    cache_grande: RefCell<HashMap<String, Option<Rc<Imagen>>>>,
    /// La flecha dibujada a mano, por si el sistema no tiene tema ninguno.
    reserva: Rc<Imagen>,
}

impl CursorTheme {
    /// Prepara el tema para una escala y un tamaño lógico dados.
    ///
    /// El límite de arriba tiene una razón medida: por encima de 128 el cursor
    /// deja de caber en el plano de hardware y pasa a componerse en la GPU,
    /// con lo que mover el ratón vuelve a costar un frame entero.
    // 0.1.0
    // modified 0.1.1 -- el nombre del tema sale de nombre_tema()
    pub fn con_tamano(scale: f64, nominal: u32) -> Self {
        let nominal = nominal.clamp(8, 128);
        let objetivo = (nominal as f64 * scale).round().max(1.0) as u32;
        // `XCURSOR_THEME` manda si está puesta: es lo que espera cualquiera que
        // venga de configurar el cursor por las vías de siempre.
        let nombre = nombre_tema();
        let theme = Self {
            nombre,
            nominal,
            objetivo,
            cache: RefCell::new(HashMap::new()),
            cache_grande: RefCell::new(HashMap::new()),
            reserva: Rc::new(fallback_arrow(objetivo, nominal)),
        };
        // Se comprueba al arrancar y no en el primer frame: si el tema no está
        // instalado, es mejor enterarse por el log al iniciar sesión que
        // preguntarse por qué el cursor se ve como el de 1995.
        if theme.buscar("left_ptr").is_none() {
            tracing::warn!(
                tema = theme.nombre,
                "sin tema de cursor; se usa la flecha de reserva"
            );
        }
        theme
    }

    /// El nombre del tema en uso, para pasárselo a los clientes.
    // 0.1.0
    pub fn nombre(&self) -> &str {
        &self.nombre
    }

    /// El tamaño **lógico** que hay que decirle a un cliente.
    ///
    /// `XCURSOR_SIZE` va en lógicos: el cliente lo multiplica él por la escala
    /// de la pantalla. Pasarle el físico da un cursor del doble de grande.
    // 0.1.0
    pub fn tamano_logico(&self) -> u32 {
        self.nominal
    }

    /// Un cursor por su nombre, cargándolo la primera vez.
    ///
    /// Devuelve `None` si no existe ni con alias: el llamante decide si eso
    /// merece la flecha de reserva o no dibujar nada.
    // 0.1.0
    fn buscar(&self, nombre: &str) -> Option<Rc<Imagen>> {
        self.buscar_en(&self.cache, nombre, self.objetivo)
    }

    fn buscar_grande(&self, nombre: &str) -> Option<Rc<Imagen>> {
        let objetivo_grande =
            (self.objetivo as f64 * f64::from(AgitarCursor::ESCALA_GRANDE))
                .round()
                .max(1.0) as u32;
        self.buscar_en(&self.cache_grande, nombre, objetivo_grande)
    }

    fn buscar_en(
        &self,
        cache: &RefCell<HashMap<String, Option<Rc<Imagen>>>>,
        nombre: &str,
        objetivo: u32,
    ) -> Option<Rc<Imagen>> {
        if let Some(hallado) = cache.borrow().get(nombre) {
            return hallado.clone();
        }
        // Primero el nombre tal cual, después su equivalente. El tema de BookOS
        // usa los nombres CSS (`text`, `pointer`, `ns-resize`) y los clientes
        // piden indistintamente esos o los de X11 (`xterm`, `hand2`,
        // `sb_v_double_arrow`), según de qué toolkit vengan.
        let imagen = std::iter::once(nombre)
            .chain(equivalentes(nombre).iter().copied())
            .find_map(|n| cargar(&self.nombre, n, objetivo, self.nominal))
            .map(Rc::new);
        if imagen.is_none() {
            tracing::debug!(nombre, "cursor que el tema no tiene");
        }
        cache
            .borrow_mut()
            .insert(nombre.to_string(), imagen.clone());
        imagen
    }

    fn element_con_calidad(
        &self,
        renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        nombre: &str,
        scale: f64,
        alta_resolucion: bool,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let imagen = if alta_resolucion {
            self.buscar_grande(nombre)
                .or_else(|| (nombre != "default").then(|| self.buscar_grande("default")).flatten())
        } else {
            self.buscar(nombre)
                .or_else(|| (nombre != "default").then(|| self.buscar("default")).flatten())
        }
        .unwrap_or_else(|| self.reserva.clone());
        // El hotspot está en píxeles del fichero; al usar una variante de mayor
        // resolución hay que llevarlo a píxeles físicos de pantalla.
        let factor_hotspot = (self.nominal as f64 * scale) / imagen.del_fichero.max(1) as f64;
        let location = (
            location.x - imagen.hotspot.0 as f64 * factor_hotspot,
            location.y - imagen.hotspot.1 as f64 * factor_hotspot,
        );
        // El `src` es el buffer entero, en sus propios píxeles: el buffer se
        // creó con escala 1, así que ahí un "lógico" es un píxel del fichero.
        // Y el `size` es lo que ocupa en pantalla, en lógicos. Los dos juntos
        // son "coge toda la imagen y ponla a este tamaño".
        let src = Rectangle::from_size((imagen.pixeles.0 as f64, imagen.pixeles.1 as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            &imagen.buffer,
            None,
            Some(src),
            Some(imagen.logico.into()),
            Kind::Cursor,
        )
        .ok()
    }
}

/// Enseña qué fichero resuelve cada cursor que el escritorio usa de verdad.
///
/// Existe por lo mismo que `--drm-info`: cuando el cursor sale mal, la pregunta
/// es si el tema no está instalado, si le falta ese nombre o si lo estamos
/// pidiendo con el nombre equivocado, y las tres se parecen desde fuera.
// 0.1.0
// modified 0.1.1 -- el nombre del tema sale de nombre_tema()
pub fn info() -> anyhow::Result<()> {
    let tema = nombre_tema();
    println!("tema: {tema}\n");

    // Los nombres tal y como los piden los clientes, no los del tema.
    const USADOS: &[&str] = &[
        "default",
        "left_ptr",
        "text",
        "xterm",
        "pointer",
        "hand2",
        "wait",
        "watch",
        "progress",
        "move",
        "fleur",
        "ew-resize",
        "e-resize",
        "w-resize",
        "sb_h_double_arrow",
        "ns-resize",
        "n-resize",
        "s-resize",
        "sb_v_double_arrow",
        "nwse-resize",
        "nw-resize",
        "se-resize",
        "nesw-resize",
        "ne-resize",
        "sw-resize",
        "top_left_corner",
        "help",
        "not-allowed",
        "grab",
        "grabbing",
        "crosshair",
        "copy",
        "col-resize",
        "row-resize",
        "zoom-in",
        "pencil",
    ];
    let (mut hallados, mut faltan) = (0, Vec::new());
    for nombre in USADOS {
        let cargado = xcursor::CursorTheme::load(&tema);
        let directo = cargado.load_icon(nombre);
        let por_alias = equivalentes(nombre)
            .iter()
            .find_map(|alias| cargado.load_icon(alias));
        match directo.or(por_alias) {
            Some(path) => {
                hallados += 1;
                println!("  {nombre:<20} {}", path.display());
            }
            None => {
                faltan.push(*nombre);
                println!("  {nombre:<20} — no está");
            }
        }
    }
    println!("\n{hallados} de {} resueltos", USADOS.len());
    if !faltan.is_empty() {
        println!("faltan: {}", faltan.join(", "));
    }
    Ok(())
}

/// Otros nombres con los que distintos temas (CSS, X11, Breeze/KDE) conocen un
/// mismo cursor.
///
/// La lista cubre los que los clientes Wayland (`wp_cursor_shape_v1`) y X11
/// piden de verdad, incluidos los ocho bordes/esquinas de redimensionado
/// (`n-resize`, `s-resize`, `e-resize`, `w-resize`, `nw-resize`, `se-resize`,
/// `ne-resize`, `sw-resize`).
// 0.1.0
// modified 0.2.4 -- devuelve múltiples alias (CSS, X11, Breeze) y cubre los 8 bordes de redimensionado
fn equivalentes(nombre: &str) -> &'static [&'static str] {
    match nombre {
        "default" | "arrow" => &["left_ptr"],
        "left_ptr" => &["default"],
        "text" | "ibeam" => &["xterm"],
        "xterm" => &["text"],
        "pointer" | "pointing_hand" => &["hand2", "hand1"],
        "hand1" | "hand2" => &["pointer"],
        "wait" => &["watch"],
        "watch" => &["wait"],
        "progress" => &["left_ptr_watch"],
        "left_ptr_watch" => &["progress"],
        "move" | "all-scroll" => &["fleur", "size_all"],
        "fleur" | "size_all" => &["move", "all-scroll"],
        "ew-resize" | "col-resize" | "e-resize" | "w-resize" | "left_side" | "right_side"
        | "sb_h_double_arrow" | "h_double_arrow" | "size_hor" => &[
            "ew-resize",
            "sb_h_double_arrow",
            "size_hor",
            "h_double_arrow",
            "e-resize",
            "w-resize",
            "left_side",
            "right_side",
            "col-resize",
        ],
        "ns-resize" | "row-resize" | "n-resize" | "s-resize" | "top_side" | "bottom_side"
        | "sb_v_double_arrow" | "v_double_arrow" | "size_ver" => &[
            "ns-resize",
            "sb_v_double_arrow",
            "size_ver",
            "v_double_arrow",
            "n-resize",
            "s-resize",
            "top_side",
            "bottom_side",
            "row-resize",
        ],
        "nwse-resize" | "nw-resize" | "se-resize" | "size_fdiag" | "fd_double_arrow"
        | "top_left_corner" | "bottom_right_corner" => &[
            "nwse-resize",
            "size_fdiag",
            "fd_double_arrow",
            "top_left_corner",
            "bottom_right_corner",
            "nw-resize",
            "se-resize",
        ],
        "nesw-resize" | "ne-resize" | "sw-resize" | "size_bdiag" | "bd_double_arrow"
        | "top_right_corner" | "bottom_left_corner" => &[
            "nesw-resize",
            "size_bdiag",
            "bd_double_arrow",
            "top_right_corner",
            "bottom_left_corner",
            "ne-resize",
            "sw-resize",
        ],
        "help" | "whats_this" => &["question_arrow"],
        "question_arrow" => &["help"],
        "not-allowed" | "forbidden" => &["crossed_circle", "no-drop"],
        "crossed_circle" | "no-drop" => &["not-allowed", "forbidden"],
        "grabbing" | "closedhand" => &["grab", "dnd-move"],
        "grab" | "openhand" => &["hand1", "pointer"],
        "crosshair" => &["cross", "tcross"],
        "cross" => &["crosshair"],
        "copy" => &["dnd-copy", "plus"],
        _ => &[],
    }
}

/// Saca una imagen del tema del sistema, a la resolución más cercana a la que
/// hace falta.
// 0.1.0
fn cargar(tema: &str, nombre: &str, objetivo: u32, nominal: u32) -> Option<Imagen> {
    let path = xcursor::CursorTheme::load(tema).load_icon(nombre)?;

    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .read_to_end(&mut bytes)
        .ok()?;
    let images = xcursor::parser::parse_xcursor(&bytes)?;

    // La imagen cuyo tamaño se acerque más al que queremos: ampliar un cursor
    // pequeño en una pantalla de 2880×1800 se ve fatal.
    let image = images
        .iter()
        .min_by_key(|img| img.size.abs_diff(objetivo))?;

    // XCursor entrega ARGB premultiplicado en orden nativo, que en
    // little-endian son bytes B,G,R,A: eso es Argb8888.
    let buffer = MemoryRenderBuffer::from_slice(
        &image.pixels_rgba,
        Fourcc::Argb8888,
        (image.width as i32, image.height as i32),
        1,
        Transform::Normal,
        None,
    );
    Some(Imagen {
        buffer,
        logico: a_logico((image.width, image.height), image.size, nominal),
        pixeles: (image.width as i32, image.height as i32),
        del_fichero: image.size,
        hotspot: (image.xhot as i32, image.yhot as i32),
    })
}

/// De píxeles del fichero a píxeles lógicos, que es en lo que razona la escena.
///
/// La conversión va por el **nominal del fichero**, no por la escala de la
/// pantalla. Una imagen de nominal 48 está pensada para un cursor de 48
/// unidades: para enseñarla como un cursor de `nominal` hay que llevarla en esa
/// proporción, y las dimensiones reales (42×48 en la flecha de BookOS) siguen
/// al lienzo.
///
/// Dividir por la escala, que es lo que hacía antes, solo acierta cuando el
/// tema tiene justo la imagen pedida. Medido: con `cursor = 48` a escala 1,75
/// se piden 84 px, el tema llega hasta 64 y salía un cursor de 39×54 px —un
/// 24% más pequeño de lo pedido— porque se dibujaba a resolución nativa en vez
/// de al tamaño que se había pedido.
// 0.1.0
fn a_logico((w, h): (u32, u32), del_fichero: u32, nominal: u32) -> (i32, i32) {
    let factor = nominal as f64 / del_fichero.max(1) as f64;
    (
        (w as f64 * factor).round().max(1.0) as i32,
        (h as f64 * factor).round().max(1.0) as i32,
    )
}

/// Flecha mínima dibujada a mano, por si el sistema no tiene tema de cursores.
// 0.1.0
fn fallback_arrow(size: u32, nominal: u32) -> Imagen {
    let size = size.max(8);
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            // Triángulo rectángulo con la punta en (0,0), como un puntero clásico.
            let dentro = x <= y && (x + y) < size;
            if !dentro {
                continue;
            }
            let borde = x == y || x == 0 || (x + y + 1) >= size;
            let i = ((y * size + x) * 4) as usize;
            // Argb8888 en little-endian = bytes B, G, R, A, premultiplicado.
            let v = if borde { 0 } else { 255 };
            pixels[i] = v;
            pixels[i + 1] = v;
            pixels[i + 2] = v;
            pixels[i + 3] = 255;
        }
    }
    Imagen {
        buffer: MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (size as i32, size as i32),
            1,
            Transform::Normal,
            None,
        ),
        // La reserva se dibuja ya al tamaño físico pedido, así que su
        // "nominal de fichero" es ese mismo tamaño.
        logico: a_logico((size, size), size, nominal),
        pixeles: (size as i32, size as i32),
        del_fichero: size,
        hotspot: (0, 0),
    }
}

/// Detector de «agitar el ratón para encontrar el cursor» (estilo macOS).
///
/// Cuenta inversiones rápidas y amplias de sentido en cada eje dentro de una
/// ventana de 650 ms. Hace falta una sacudida sostenida (≥ 7 inversiones en el
/// mismo eje y ≥ 850 px lógicos de recorrido) para agrandar el cursor hasta
/// 2,5× y volver suavemente a 1,0× al parar.
// añadido en la 0.2.4
pub struct AgitarCursor {
    pub habilitado: bool,
    ultima_pos: Option<(f64, f64)>,
    dir_x: i8,
    dir_y: i8,
    contra_x: f64,
    contra_y: f64,
    inversiones: Vec<(std::time::Instant, f64, bool, bool)>,
    hasta: Option<std::time::Instant>,
    escala: bookos_shell::tema::Muelle,
}

impl AgitarCursor {
    pub(crate) const ESCALA_GRANDE: f32 = 2.5;
    const VENTANA: std::time::Duration = std::time::Duration::from_millis(650);
    const MANTENER: std::time::Duration = std::time::Duration::from_millis(400);
    /// Distancia mínima en sentido opuesto (lógicos) para contar una inversión
    /// real y evitar que movimientos cortos o micro-desvíos disparen el zoom.
    const UMBRAL_GIRO: f64 = 45.0;
    const MIN_INVERSIONES: usize = 7;
    const RECORRIDO_MINIMO: f64 = 850.0;

    // añadido en la 0.2.4
    pub fn nuevo(habilitado: bool) -> Self {
        Self {
            habilitado,
            ultima_pos: None,
            dir_x: 0,
            dir_y: 0,
            contra_x: 0.0,
            contra_y: 0.0,
            inversiones: Vec::new(),
            hasta: None,
            escala: bookos_shell::tema::Muelle::nuevo(1.0, bookos_shell::tema::M_RAPIDO),
        }
    }

    /// Registra un movimiento del puntero en coordenadas lógicas.
    /// Devuelve `true` si arrancó o renovó el agrandamiento.
    // añadido en la 0.2.4
    pub fn mover(&mut self, x: f64, y: f64, ahora: std::time::Instant) -> bool {
        if !self.habilitado {
            return false;
        }
        let Some((px, py)) = self.ultima_pos.replace((x, y)) else {
            return false;
        };
        let dx = x - px;
        let dy = y - py;
        let dist = dx.hypot(dy);
        if dist < 1.0 {
            return false;
        }

        let invirtio_x = actualizar_eje(dx, &mut self.dir_x, &mut self.contra_x, Self::UMBRAL_GIRO);
        let invirtio_y = actualizar_eje(dy, &mut self.dir_y, &mut self.contra_y, Self::UMBRAL_GIRO);

        self.inversiones
            .retain(|(t, _, _, _)| ahora.duration_since(*t) <= Self::VENTANA);
        if invirtio_x || invirtio_y {
            self.inversiones
                .push((ahora, dist, invirtio_x, invirtio_y));
        } else if let Some(ultima) = self.inversiones.last_mut() {
            ultima.1 += dist;
        }

        let mut inv_x = 0usize;
        let mut inv_y = 0usize;
        let mut recorrido = 0.0f64;
        for &(_, d, ix, iy) in &self.inversiones {
            recorrido += d;
            if ix {
                inv_x += 1;
            }
            if iy {
                inv_y += 1;
            }
        }
        if inv_x.max(inv_y) >= Self::MIN_INVERSIONES && recorrido >= Self::RECORRIDO_MINIMO {
            let antes = self.hasta.is_some();
            self.hasta = Some(ahora + Self::MANTENER);
            self.escala.poner_parametros(bookos_shell::tema::M_RAPIDO);
            self.escala.ir_a(Self::ESCALA_GRANDE);
            return !antes;
        }
        false
    }

    // añadido en la 0.2.4
    pub fn avanzar(&mut self, ahora: std::time::Instant) {
        if let Some(hasta) = self.hasta
            && ahora >= hasta
        {
            self.hasta = None;
            // Vuelta suave y progresiva al tamaño normal.
            self.escala.poner_parametros(bookos_shell::tema::M_SUAVE);
            self.escala.ir_a(1.0);
        }
        self.escala.avanzar(ahora);
    }

    // añadido en la 0.2.4
    pub fn escala(&self) -> f64 {
        f64::from(self.escala.valor()).clamp(1.0, 3.0)
    }

    // añadido en la 0.2.4
    pub fn animando(&self) -> bool {
        self.hasta.is_some() || !self.escala.quieto() || self.escala.valor() > 1.002
    }
}

fn actualizar_eje(delta: f64, dir: &mut i8, contra: &mut f64, umbral: f64) -> bool {
    if delta.abs() < 0.5 {
        return false;
    }
    let signo = if delta > 0.0 { 1 } else { -1 };
    if *dir == 0 {
        *contra += delta.abs();
        if *contra >= 6.0 {
            *dir = signo;
            *contra = 0.0;
        }
        return false;
    }
    if signo == *dir {
        *contra = 0.0;
        false
    } else {
        *contra += delta.abs();
        if *contra >= umbral {
            *dir = signo;
            *contra = 0.0;
            true
        } else {
            false
        }
    }
}

/// Construye los elementos del cursor para este frame.
///
/// Devuelve vacío cuando el cursor está oculto: hay clientes que lo esconden a
/// propósito (una terminal mientras escribes, un vídeo a pantalla completa) y
/// respetarlo es parte de comportarse como un escritorio.
// 0.1.0
// modified 0.1.1 -- amplía el cursor de los programas X11 a la escala de XWayland
// modified 0.2.4 -- aplica `escala_extra` cuando se agita el ratón para localizarlo
pub fn elements(
    renderer: &mut GlesRenderer,
    status: &smithay::input::pointer::CursorImageStatus,
    theme: Option<&CursorTheme>,
    location: Point<f64, Physical>,
    scale: f64,
    escala_extra: f64,
) -> Vec<OverlayElement> {
    use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
    use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
    use smithay::wayland::compositor::with_states;

    let ampliar = escala_extra > 1.002;
    let origen_escala = location.to_i32_round();

    match status {
        CursorImageStatus::Hidden => Vec::new(),
        // El nombre **se respeta**: antes se dibujaba la flecha pasara lo que
        // pasara, así que al acercarse al borde de una ventana el cursor no
        // cambiaba a la doble flecha de redimensionar y no había forma de saber
        // que ahí se podía tirar.
        CursorImageStatus::Named(nombre) => theme
            .and_then(|theme| {
                theme.element_con_calidad(renderer, location, nombre.name(), scale, ampliar)
            })
            .map(|el| {
                if ampliar {
                    OverlayElement::MemoriaEscalada(RescaleRenderElement::from_element(
                        el,
                        origen_escala,
                        escala_extra,
                    ))
                } else {
                    OverlayElement::Memory(el)
                }
            })
            .into_iter()
            .collect(),
        CursorImageStatus::Surface(surface) => {
            // El hotspot lo fija el cliente y está en coordenadas lógicas de su
            // superficie; hay que escalarlo antes de restarlo en físicas.
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .map(|d| d.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            // Un programa X11 carga el cursor a `XCURSOR_SIZE` píxeles sin
            // saber de escalas, y con XWayland en físicos eso es la mitad de
            // grande en un panel a 2. Se amplía aquí: el entorno es el mismo
            // para clientes X11 y Wayland, así que no se le puede dar a unos
            // un tamaño y a otros otro.
            use smithay::reexports::wayland_server::Resource;
            let escala_x11 = surface
                .client()
                .and_then(|c| {
                    c.get_data::<smithay::xwayland::XWaylandClientData>()
                        .map(|d| d.compositor_state.client_scale())
                })
                .unwrap_or(1.0);
            let origin = (
                location.x - hotspot.x as f64 * scale,
                location.y - hotspot.y as f64 * scale,
            );
            // El tipo del elemento hay que decirlo: el helper es genérico y aquí
            // se envuelve en nuestro enum.
            let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    renderer,
                    surface,
                    Point::<f64, Physical>::from(origin).to_i32_round(),
                    scale,
                    1.0,
                    Kind::Cursor,
                );
            let factor = escala_x11 * if ampliar { escala_extra } else { 1.0 };
            elements
                .into_iter()
                .map(|elemento| {
                    if factor <= 1.01 {
                        OverlayElement::Surface(elemento)
                    } else {
                        // La escala de `render_elements_from_surface_tree` solo
                        // coloca las superficies. El tamaño se calcula después
                        // con la escala de salida: hay que ampliar el elemento,
                        // incluido el hotspot, alrededor del punto del ratón.
                        OverlayElement::Escalada(RescaleRenderElement::from_element(
                            elemento,
                            origen_escala,
                            factor,
                        ))
                    }
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests_agitar {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn agitar_agranda_y_vuelve_a_uno_al_parar() {
        let mut ag = AgitarCursor::nuevo(true);
        let t0 = Instant::now();
        // Sacudida intensa: 9 puntos (8 tramos de ~130 px, 7 inversiones en X).
        let puntos = [
            (100.0, 100.0),
            (230.0, 100.0),
            (100.0, 100.0),
            (230.0, 100.0),
            (100.0, 100.0),
            (230.0, 100.0),
            (100.0, 100.0),
            (230.0, 100.0),
            (100.0, 100.0),
        ];
        for (i, (x, y)) in puntos.into_iter().enumerate() {
            ag.mover(x, y, t0 + Duration::from_millis(i as u64 * 45));
        }
        assert!(ag.animando(), "agitar mucho debe disparar el agrandamiento");
        ag.avanzar(t0 + Duration::from_millis(450));
        assert!(ag.escala() > 1.5, "escala actual: {}", ag.escala());
        // Pasado el tiempo de espera + asentamiento del muelle vuelve a 1.0.
        ag.avanzar(t0 + Duration::from_secs(2));
        ag.avanzar(t0 + Duration::from_secs(3));
        assert!((ag.escala() - 1.0).abs() < 0.05);
    }

    #[test]
    fn agitar_poco_o_mover_en_cruz_no_agranda() {
        let mut ag = AgitarCursor::nuevo(true);
        let t0 = Instant::now();
        // Solo dos idas y vueltas moderadas: no debe disparar.
        let puntos = [
            (100.0, 100.0),
            (180.0, 100.0),
            (100.0, 100.0),
            (180.0, 100.0),
            (100.0, 100.0),
        ];
        for (i, (x, y)) in puntos.into_iter().enumerate() {
            assert!(
                !ag.mover(x, y, t0 + Duration::from_millis(i as u64 * 45)),
                "agitar poco no debe disparar el agrandamiento"
            );
        }
        assert!(!ag.animando());
    }

    #[test]
    fn un_tiro_rapido_con_micro_desvio_no_agranda() {
        let mut ag = AgitarCursor::nuevo(true);
        let t0 = Instant::now();
        // Un único deslizamiento rápido horizontal con leve temblor vertical (±6 px).
        let puntos = [
            (100.0, 200.0),
            (180.0, 206.0),
            (270.0, 200.0),
            (360.0, 206.0),
            (450.0, 200.0),
            (540.0, 206.0),
        ];
        for (i, (x, y)) in puntos.into_iter().enumerate() {
            assert!(
                !ag.mover(x, y, t0 + Duration::from_millis(i as u64 * 16)),
                "un tiro rápido no debe disparar el agrandamiento"
            );
        }
        assert!(!ag.animando());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La flecha de BookOS, tal como está en el fichero: el nominal 48 mide
    /// 42×48 px de verdad, y el 64 mide 57×64.
    const FLECHA_48: (u32, u32) = (42, 48);
    const FLECHA_64: (u32, u32) = (57, 64);

    #[test]
    fn el_cursor_mide_lo_que_se_pide_aunque_el_tema_no_lo_tenga() {
        // El caso normal: se piden 24 lógicos y el tema tiene la imagen justa.
        // El lienzo pasa a medir 24, y el contenido lo que le toque.
        assert_eq!(a_logico(FLECHA_48, 48, 24), (21, 24));
        // El caso que salía mal: `cursor = 48` a escala 1,75 pide 84 px y el
        // tema llega hasta 64. Antes se dibujaba la de 64 a su resolución
        // nativa y quedaba un 24% pequeña; ahora se declara con el doble de
        // lógicos que el caso de arriba, que es lo que significa pedir 48.
        assert_eq!(a_logico(FLECHA_64, 64, 48), (43, 48));
    }

    #[test]
    fn el_doble_de_tamano_da_el_doble_de_cursor() {
        let (w24, h24) = a_logico(FLECHA_48, 48, 24);
        let (w48, h48) = a_logico(FLECHA_48, 48, 48);
        assert_eq!((w48, h48), (w24 * 2, h24 * 2));
    }

    #[test]
    fn una_imagen_diminuta_no_desaparece() {
        // Redondear hacia abajo daría 0, y un elemento de tamaño cero no se
        // dibuja: cursor invisible.
        assert_eq!(a_logico((1, 1), 64, 8), (1, 1));
    }

    #[test]
    fn los_ocho_bordes_de_redimensionado_tienen_alias_css_x11_y_breeze() {
        for vertical in ["n-resize", "s-resize", "ns-resize"] {
            let alias = equivalentes(vertical);
            assert!(alias.contains(&"ns-resize"));
            assert!(alias.contains(&"sb_v_double_arrow"));
            assert!(alias.contains(&"size_ver"));
        }
        for horizontal in ["e-resize", "w-resize", "ew-resize"] {
            let alias = equivalentes(horizontal);
            assert!(alias.contains(&"ew-resize"));
            assert!(alias.contains(&"sb_h_double_arrow"));
            assert!(alias.contains(&"size_hor"));
        }
        for fdiag in ["nw-resize", "se-resize", "nwse-resize"] {
            let alias = equivalentes(fdiag);
            assert!(alias.contains(&"nwse-resize"));
            assert!(alias.contains(&"size_fdiag"));
            assert!(alias.contains(&"top_left_corner"));
        }
        for bdiag in ["ne-resize", "sw-resize", "nesw-resize"] {
            let alias = equivalentes(bdiag);
            assert!(alias.contains(&"nesw-resize"));
            assert!(alias.contains(&"size_bdiag"));
            assert!(alias.contains(&"top_right_corner"));
        }
    }
}
