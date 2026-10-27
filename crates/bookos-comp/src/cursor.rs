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
    ///
    /// Hace falta para decirle a `from_buffer` qué trozo del buffer hay que
    /// pintar. Si no se le dice, **usa el tamaño lógico como recorte**: con un
    /// fichero de 42×48 dibujado a 24×27 lógicos salían solo los 24×27 píxeles
    /// de la esquina superior izquierda, estirados. En pantalla era media
    /// flecha — el borde izquierdo y el filo de arriba, sin punta ni relleno.
    pixeles: (i32, i32),
    /// Punto "activo" dentro de la imagen: la punta de la flecha, en píxeles
    /// físicos, que es donde se resta.
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
    ///
    /// `RefCell` porque esto es una caché y no un cambio de estado: el frame se
    /// dibuja con `&self` desde la escena, y pedir `&mut` allí obligaría a sacar
    /// el tema del compositor y volver a meterlo en cada fotograma.
    cache: RefCell<HashMap<String, Option<Rc<Imagen>>>>,
    /// La flecha dibujada a mano, por si el sistema no tiene tema ninguno.
    reserva: Rc<Imagen>,
}

impl CursorTheme {
    /// Prepara el tema para una escala y un tamaño lógico dados.
    ///
    /// El límite de arriba tiene una razón medida: por encima de 128 el cursor
    /// deja de caber en el plano de hardware y pasa a componerse en la GPU,
    /// con lo que mover el ratón vuelve a costar un frame entero.
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
    pub fn nombre(&self) -> &str {
        &self.nombre
    }

    /// El tamaño **lógico** que hay que decirle a un cliente.
    ///
    /// `XCURSOR_SIZE` va en lógicos: el cliente lo multiplica él por la escala
    /// de la pantalla. Pasarle el físico da un cursor del doble de grande.
    pub fn tamano_logico(&self) -> u32 {
        self.nominal
    }

    /// Un cursor por su nombre, cargándolo la primera vez.
    ///
    /// Devuelve `None` si no existe ni con alias: el llamante decide si eso
    /// merece la flecha de reserva o no dibujar nada.
    fn buscar(&self, nombre: &str) -> Option<Rc<Imagen>> {
        if let Some(hallado) = self.cache.borrow().get(nombre) {
            return hallado.clone();
        }
        // Primero el nombre tal cual, después su equivalente. El tema de BookOS
        // usa los nombres CSS (`text`, `pointer`, `ns-resize`) y los clientes
        // piden indistintamente esos o los de X11 (`xterm`, `hand2`,
        // `sb_v_double_arrow`), según de qué toolkit vengan.
        let imagen = std::iter::once(nombre)
            .chain(equivalente(nombre))
            .find_map(|n| cargar(&self.nombre, n, self.objetivo, self.nominal))
            .map(Rc::new);
        if imagen.is_none() {
            tracing::debug!(nombre, "cursor que el tema no tiene");
        }
        self.cache
            .borrow_mut()
            .insert(nombre.to_string(), imagen.clone());
        imagen
    }

    pub fn element(
        &self,
        renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        nombre: &str,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let imagen = self.buscar(nombre).unwrap_or_else(|| self.reserva.clone());
        // El hotspot se descuenta de la posición: la punta de la flecha tiene
        // que caer donde está el puntero, no la esquina de la imagen.
        let location = (
            location.x - imagen.hotspot.0 as f64,
            location.y - imagen.hotspot.1 as f64,
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
        "sb_h_double_arrow",
        "ns-resize",
        "sb_v_double_arrow",
        "nwse-resize",
        "nesw-resize",
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
        let directo = xcursor::CursorTheme::load(&tema).load_icon(nombre);
        let por_alias = equivalente(nombre)
            .and_then(|alias| xcursor::CursorTheme::load(&tema).load_icon(alias));
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

/// El otro nombre con el que se conoce un cursor.
///
/// La lista no es exhaustiva a propósito: cubre los que un escritorio usa de
/// verdad. Lo que falte cae en el `Inherits` del tema o, en último caso, en la
/// flecha — que es mejor que un cursor invisible.
fn equivalente(nombre: &str) -> Option<&'static str> {
    Some(match nombre {
        "default" | "arrow" => "left_ptr",
        "left_ptr" => "default",
        "text" | "ibeam" => "xterm",
        "xterm" => "text",
        "pointer" | "pointing_hand" => "hand2",
        "hand1" | "hand2" => "pointer",
        "wait" => "watch",
        "watch" => "wait",
        "progress" => "left_ptr_watch",
        "left_ptr_watch" => "progress",
        "move" | "all-scroll" => "fleur",
        "fleur" | "size_all" => "move",
        "ew-resize" | "col-resize" => "sb_h_double_arrow",
        "sb_h_double_arrow" | "h_double_arrow" => "ew-resize",
        "ns-resize" | "row-resize" => "sb_v_double_arrow",
        "sb_v_double_arrow" | "v_double_arrow" => "ns-resize",
        "nwse-resize" | "size_fdiag" => "bottom_right_corner",
        "top_left_corner" | "bottom_right_corner" => "nwse-resize",
        "nesw-resize" | "size_bdiag" => "bottom_left_corner",
        "top_right_corner" | "bottom_left_corner" => "nesw-resize",
        "help" | "whats_this" => "question_arrow",
        "question_arrow" => "help",
        "not-allowed" | "forbidden" => "crossed_circle",
        "crossed_circle" | "no-drop" => "not-allowed",
        "grabbing" | "closedhand" => "grab",
        "grab" | "openhand" => "hand1",
        "crosshair" => "cross",
        "cross" => "crosshair",
        "copy" => "dnd-copy",
        _ => return None,
    })
}

/// Saca una imagen del tema del sistema, a la resolución más cercana a la que
/// hace falta.
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
fn a_logico((w, h): (u32, u32), del_fichero: u32, nominal: u32) -> (i32, i32) {
    let factor = nominal as f64 / del_fichero.max(1) as f64;
    (
        (w as f64 * factor).round().max(1.0) as i32,
        (h as f64 * factor).round().max(1.0) as i32,
    )
}

/// Flecha mínima dibujada a mano, por si el sistema no tiene tema de cursores.
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
        hotspot: (0, 0),
    }
}

/// Construye los elementos del cursor para este frame.
///
/// Devuelve vacío cuando el cursor está oculto: hay clientes que lo esconden a
/// propósito (una terminal mientras escribes, un vídeo a pantalla completa) y
/// respetarlo es parte de comportarse como un escritorio.
pub fn elements(
    renderer: &mut GlesRenderer,
    status: &smithay::input::pointer::CursorImageStatus,
    theme: Option<&CursorTheme>,
    location: Point<f64, Physical>,
    scale: f64,
) -> Vec<OverlayElement> {
    use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
    use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
    use smithay::wayland::compositor::with_states;

    match status {
        CursorImageStatus::Hidden => Vec::new(),
        // El nombre **se respeta**: antes se dibujaba la flecha pasara lo que
        // pasara, así que al acercarse al borde de una ventana el cursor no
        // cambiaba a la doble flecha de redimensionar y no había forma de saber
        // que ahí se podía tirar.
        CursorImageStatus::Named(nombre) => theme
            .and_then(|theme| theme.element(renderer, location, nombre.name()))
            .map(OverlayElement::Memory)
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
            elements
                .into_iter()
                .map(|elemento| {
                    if escala_x11 == 1.0 {
                        OverlayElement::Surface(elemento)
                    } else {
                        // La escala de `render_elements_from_surface_tree` solo
                        // coloca las superficies. El tamaño se calcula después
                        // con la escala de salida: hay que ampliar el elemento,
                        // incluido el hotspot, alrededor del punto del ratón.
                        OverlayElement::Escalada(RescaleRenderElement::from_element(
                            elemento,
                            location.to_i32_round(),
                            escala_x11,
                        ))
                    }
                })
                .collect()
        }
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
}
