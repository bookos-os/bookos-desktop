//! Encontrar y cargar los iconos de las aplicaciones.
//!
//! Lo comparten el dock y el launchpad. La búsqueda es la de freedesktop hecha
//! a mano —producto de directorios por temas por tamaños por categorías— y por
//! eso **no es gratis**: unos cientos de `stat` por icono que no existe. El
//! dock resuelve cinco al arrancar y no se nota; el launchpad tiene que
//! resolver los de la página que se está viendo y no los de las trescientas
//! aplicaciones que hay instaladas.

use std::path::{Path, PathBuf};

use iced_widget::{image as iced_image, svg};

/// Un icono ya cargado, listo para dibujar. Clonarlo es barato —los `Handle` de iced son punteros
/// contados— y además **conserva el identificador**, que es lo que permite a
/// iced reutilizar el rasterizado. Ver [`CACHE`].
#[derive(Clone)]
pub enum Icono {
    Svg(svg::Handle),
    Raster(iced_image::Handle),
}

/// El icono como elemento, al tamaño y con el color que se le pidan.
///
/// Son SVG, que resvg rasteriza al tamaño que se le pida: por eso da igual que
/// el fichero esté en la carpeta de 16 o en la de 32.
///
/// Los iconos de estado son monocromos y su color va dentro del fichero, así
/// que dependen de qué variante del tema esté instalada. Tiñéndolos aquí el
/// panel manda sobre el tema de iconos y no al revés — que es lo que hace falta
/// para que un icono siga al color del texto cuando la batería se pone en rojo.
/// Los de aplicación **no** se tiñen: convertir el zorro de Firefox en una
/// silueta blanca no es lo que nadie quiere.
pub fn ver_teñido(
    icono: &Icono,
    px: f32,
    color: Option<iced_core::Color>,
) -> crate::view::PanelElement<'_> {
    use iced_core::Length;
    use iced_widget::{image as iced_image, svg};
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .style(move |_theme, _status| svg::Style { color })
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .into(),
    }
}

/// Como [`ver_teñido`], pero sin atar el resultado al icono.
///
/// Hace falta donde el icono es temporal —los menús lo piden a [`propio`] al
/// dibujar cada fila— y `ver_teñido` no vale porque su firma lo ata al préstamo
/// aunque por dentro solo clone el `Handle`, que es lo único que el elemento
/// necesita de verdad.
pub fn ver_teñido_propio(
    icono: &Icono,
    px: f32,
    color: iced_core::Color,
) -> crate::view::PanelElement<'static> {
    use iced_core::Length;
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .style(move |_theme, _status| svg::Style { color: Some(color) })
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .into(),
    }
}

/// Como [`ver_teñido`], con opacidad: lo que necesita un icono a medio cruzar.
pub fn ver_escalado(
    icono: &Icono,
    px: f32,
    opacidad: f32,
    color: iced_core::Color,
) -> crate::view::PanelElement<'static> {
    use iced_core::Length;
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .opacity(opacidad)
            .style(move |_theme, _status| svg::Style { color: Some(color) })
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .opacity(opacidad)
            .into(),
    }
}

/// Un icono a partir de un SVG construido en memoria.
///
/// Lo usan los widgets que se dibujan solos —la batería— en vez de tirar del
/// tema: el pictograma cambia de relleno y de color con el estado, y un tema de
/// iconos solo tiene escalones fijos.
pub fn desde_svg(svg: &str) -> Icono {
    Icono::Svg(svg::Handle::from_memory(svg.as_bytes().to_vec()))
}

/// El icono a un tamaño que no tiene por qué ser cuadrado, y sin teñir: lo que
/// necesita un pictograma que ya trae sus colores dentro.
pub fn ver(icono: &Icono, ancho: f32, alto: f32) -> crate::view::PanelElement<'_> {
    use iced_core::Length;
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .into(),
    }
}

/// Los iconos propios de BookOS, incrustados en el binario.
///
/// Son los mismos trazos que dibujan los plasmoides en QML —viewBox de 24,
/// grosor 2, extremos redondeados— pero como ficheros, en vez de armar un
/// `data:` URI en cada frame. Van dentro del binario por lo mismo que el logo:
/// el panel no puede quedarse sin sus propios iconos porque falte un paquete.
///
/// El color va a `#ffffff` dentro del fichero y lo tiñe [`ver_teñido`]. Con
/// `currentColor` el icono saldría sin pintar: usvg no lo resuelve sin un
/// contexto de estilo que aquí no hay.
const PROPIOS: &[(&str, &[u8])] = &[
    // Las teclas especiales del teclado en pantalla. SVG y no ⇧ ⌫ ⏎ en texto:
    // esos glifos no están en todas las fuentes y caían a otra de otro peso.
    ("mayus", include_bytes!("../assets/iconos/mayus.svg")),
    (
        "mayus-fija",
        include_bytes!("../assets/iconos/mayus-fija.svg"),
    ),
    ("borrar", include_bytes!("../assets/iconos/borrar.svg")),
    ("intro", include_bytes!("../assets/iconos/intro.svg")),
    (
        "chevron-abajo",
        include_bytes!("../assets/iconos/chevron-abajo.svg"),
    ),
    (
        "volumen-silencio",
        include_bytes!("../assets/iconos/volumen-silencio.svg"),
    ),
    (
        "volumen-bajo",
        include_bytes!("../assets/iconos/volumen-bajo.svg"),
    ),
    (
        "volumen-medio",
        include_bytes!("../assets/iconos/volumen-medio.svg"),
    ),
    (
        "volumen-alto",
        include_bytes!("../assets/iconos/volumen-alto.svg"),
    ),
    ("micro", include_bytes!("../assets/iconos/micro.svg")),
    // --- Del sistema de diseño (heroicons 24/outline) --------------------
    // Vienen de BookOS-HIG y sustituyen a los dibujados a mano: el escritorio
    // y sus aplicaciones enseñan así el mismo icono para la misma cosa.
    (
        "bateria-0",
        include_bytes!("../assets/iconos/bateria-0.svg"),
    ),
    (
        "bateria-50",
        include_bytes!("../assets/iconos/bateria-50.svg"),
    ),
    (
        "bateria-100",
        include_bytes!("../assets/iconos/bateria-100.svg"),
    ),
    (
        "bateria-carga",
        include_bytes!("../assets/iconos/bateria-carga.svg"),
    ),
    ("wifi", include_bytes!("../assets/iconos/wifi.svg")),
    ("cable", include_bytes!("../assets/iconos/cable.svg")),
    ("sin-red", include_bytes!("../assets/iconos/sin-red.svg")),
    (
        "notificaciones",
        include_bytes!("../assets/iconos/notificaciones.svg"),
    ),
    (
        "notificaciones-aviso",
        include_bytes!("../assets/iconos/notificaciones-aviso.svg"),
    ),
    ("control", include_bytes!("../assets/iconos/control.svg")),
    ("brillo", include_bytes!("../assets/iconos/brillo.svg")),
    (
        "brillo-auto",
        include_bytes!("../assets/iconos/brillo-auto.svg"),
    ),
    // El sol del panel con el automático puesto: la «A» en lugar del núcleo.
    (
        "brillo-automatico",
        include_bytes!("../assets/iconos/brillo-automatico.svg"),
    ),
    // Los dos del buscador: la lupa del campo y la terminal de un comando.
    ("buscar", include_bytes!("../assets/iconos/buscar.svg")),
    ("terminal", include_bytes!("../assets/iconos/terminal.svg")),
    ("teclado", include_bytes!("../assets/iconos/teclado.svg")),
    ("touchpad", include_bytes!("../assets/iconos/touchpad.svg")),
    ("camara", include_bytes!("../assets/iconos/camara.svg")),
    // La raya que se pone encima de un icono para decir «apagado». Va aparte y
    // no dibujada dentro de cada uno porque el OSD la tiñe de rojo mientras el
    // icono va del color del texto, y un SVG solo admite un tinte.
    ("tachado", include_bytes!("../assets/iconos/tachado.svg")),
    // El logo del sistema, que es lo que abre: el cajón de todo. Va sin teñir
    // —tiene su propia paleta— y por eso el dock lo trata como un icono de
    // aplicación y no como uno de estado.
    (
        "launchpad",
        include_bytes!("../assets/iconos/launchpad.svg"),
    ),
    // El cohete, que es el que lleva el dock de serie. El logo de arriba sigue
    // disponible con `dock = launchpad:Aplicaciones:launchpad, ...`
    (
        "launchpad-cohete",
        include_bytes!("../assets/iconos/launchpad-cohete.svg"),
    ),
    (
        "bluetooth",
        include_bytes!("../assets/iconos/bluetooth.svg"),
    ),
    (
        "bluetooth-apagado",
        include_bytes!("../assets/iconos/bluetooth-apagado.svg"),
    ),
    (
        "bluetooth-conectado",
        include_bytes!("../assets/iconos/bluetooth-conectado.svg"),
    ),
    (
        "micro-silencio",
        include_bytes!("../assets/iconos/micro-silencio.svg"),
    ),
    // --- Los de los menús ------------------------------------------------
    // Los cinco del clic derecho del escritorio, heroicons 24/outline.
    (
        "carpeta-nueva",
        include_bytes!("../assets/iconos/carpeta-nueva.svg"),
    ),
    ("fondo", include_bytes!("../assets/iconos/fondo.svg")),
    ("pantalla", include_bytes!("../assets/iconos/pantalla.svg")),
    ("papelera", include_bytes!("../assets/iconos/papelera.svg")),
    ("abrir", include_bytes!("../assets/iconos/abrir.svg")),
    ("acerca", include_bytes!("../assets/iconos/acerca.svg")),
    (
        "preferencias",
        include_bytes!("../assets/iconos/preferencias.svg"),
    ),
    (
        "apariencia",
        include_bytes!("../assets/iconos/apariencia.svg"),
    ),
    ("tienda", include_bytes!("../assets/iconos/tienda.svg")),
    ("dormir", include_bytes!("../assets/iconos/dormir.svg")),
    ("hibernar", include_bytes!("../assets/iconos/hibernar.svg")),
    (
        "reiniciar",
        include_bytes!("../assets/iconos/reiniciar.svg"),
    ),
    ("apagar", include_bytes!("../assets/iconos/apagar.svg")),
    ("bloquear", include_bytes!("../assets/iconos/bloquear.svg")),
    ("salir", include_bytes!("../assets/iconos/salir.svg")),
    (
        "ventana-nueva",
        include_bytes!("../assets/iconos/ventana-nueva.svg"),
    ),
    ("fijar", include_bytes!("../assets/iconos/fijar.svg")),
    ("cerrar", include_bytes!("../assets/iconos/cerrar.svg")),
    ("ojo", include_bytes!("../assets/iconos/ojo.svg")),
    (
        "ojo-tachado",
        include_bytes!("../assets/iconos/ojo-tachado.svg"),
    ),
    // --- Los del centro de control ---------------------------------------
    ("avion", include_bytes!("../assets/iconos/avion.svg")),
    // --- Los perfiles de energía -----------------------------------------
    // El rayo tachado, la balanza y el cohete: en la tarjeta de energía cada
    // perfil se reconoce por su dibujo y no solo por el color, que a un
    // daltónico le deja tres puntos iguales.
    (
        "perfil-ahorro",
        include_bytes!("../assets/iconos/perfil-ahorro.svg"),
    ),
    (
        "perfil-equilibrado",
        include_bytes!("../assets/iconos/perfil-equilibrado.svg"),
    ),
    (
        "perfil-rendimiento",
        include_bytes!("../assets/iconos/perfil-rendimiento.svg"),
    ),
    ("cpu", include_bytes!("../assets/iconos/cpu.svg")),
    ("ahorro", include_bytes!("../assets/iconos/ahorro.svg")),
    ("mantener", include_bytes!("../assets/iconos/mantener.svg")),
    ("noche", include_bytes!("../assets/iconos/noche.svg")),
    (
        "compartir",
        include_bytes!("../assets/iconos/compartir.svg"),
    ),
    ("captura", include_bytes!("../assets/iconos/captura.svg")),
    ("editar", include_bytes!("../assets/iconos/editar.svg")),
    ("musica", include_bytes!("../assets/iconos/musica.svg")),
    ("anterior", include_bytes!("../assets/iconos/anterior.svg")),
    (
        "siguiente",
        include_bytes!("../assets/iconos/siguiente.svg"),
    ),
    (
        "reproducir",
        include_bytes!("../assets/iconos/reproducir.svg"),
    ),
    ("pausa", include_bytes!("../assets/iconos/pausa.svg")),
    // Los dos del emergente de energía: la marca del perfil elegido y la
    // flecha del pie que lleva a los ajustes. Son `16/solid` y no `24/outline`
    // porque a 14 px un trazo de 1,5 se ve gris en vez de nítido.
    (
        "comprobado",
        include_bytes!("../assets/iconos/comprobado.svg"),
    ),
    (
        "chevron-derecha",
        include_bytes!("../assets/iconos/chevron-derecha.svg"),
    ),
    (
        "chevron-izquierda",
        include_bytes!("../assets/iconos/chevron-izquierda.svg"),
    ),
];

/// Las variantes macizas que usa el centro de control: Heroicons 2.1.5
/// `24/solid` de BookOS-HIG donde hay equivalente. Los propios se rellenan a
/// partir de ellos —el altavoz de `speaker-wave` sin ondas, el wifi tachado con
/// una máscara, el sol automático con la «A» calada en el disco— y el
/// Bluetooth, que es solo trazo, va más grueso. El avión ya era macizo y no
/// está.
///
/// Van aparte y no en lugar de los de contorno porque los nombres los comparten
/// la barra, el OSD y los menús, que siguen con trazo. La segunda columna es la
/// clave de la caché: estática, para no montar una cadena en cada fotograma.
const SOLIDOS: &[(&str, &str, &[u8])] = &[
    (
        "ahorro",
        "solido/ahorro",
        include_bytes!("../assets/iconos/solido/ahorro.svg"),
    ),
    (
        "anterior",
        "solido/anterior",
        include_bytes!("../assets/iconos/solido/anterior.svg"),
    ),
    (
        "apagar",
        "solido/apagar",
        include_bytes!("../assets/iconos/solido/apagar.svg"),
    ),
    (
        "bloquear",
        "solido/bloquear",
        include_bytes!("../assets/iconos/solido/bloquear.svg"),
    ),
    (
        "bluetooth",
        "solido/bluetooth",
        include_bytes!("../assets/iconos/solido/bluetooth.svg"),
    ),
    (
        "bluetooth-apagado",
        "solido/bluetooth-apagado",
        include_bytes!("../assets/iconos/solido/bluetooth-apagado.svg"),
    ),
    (
        "brillo",
        "solido/brillo",
        include_bytes!("../assets/iconos/solido/brillo.svg"),
    ),
    (
        "brillo-automatico",
        "solido/brillo-automatico",
        include_bytes!("../assets/iconos/solido/brillo-automatico.svg"),
    ),
    (
        "captura",
        "solido/captura",
        include_bytes!("../assets/iconos/solido/captura.svg"),
    ),
    (
        "compartir",
        "solido/compartir",
        include_bytes!("../assets/iconos/solido/compartir.svg"),
    ),
    (
        "mantener",
        "solido/mantener",
        include_bytes!("../assets/iconos/solido/mantener.svg"),
    ),
    (
        "musica",
        "solido/musica",
        include_bytes!("../assets/iconos/solido/musica.svg"),
    ),
    (
        "noche",
        "solido/noche",
        include_bytes!("../assets/iconos/solido/noche.svg"),
    ),
    (
        "notificaciones",
        "solido/notificaciones",
        include_bytes!("../assets/iconos/solido/notificaciones.svg"),
    ),
    (
        "pausa",
        "solido/pausa",
        include_bytes!("../assets/iconos/solido/pausa.svg"),
    ),
    (
        "preferencias",
        "solido/preferencias",
        include_bytes!("../assets/iconos/solido/preferencias.svg"),
    ),
    (
        "reproducir",
        "solido/reproducir",
        include_bytes!("../assets/iconos/solido/reproducir.svg"),
    ),
    (
        "siguiente",
        "solido/siguiente",
        include_bytes!("../assets/iconos/solido/siguiente.svg"),
    ),
    (
        "sin-red",
        "solido/sin-red",
        include_bytes!("../assets/iconos/solido/sin-red.svg"),
    ),
    (
        "volumen-alto",
        "solido/volumen-alto",
        include_bytes!("../assets/iconos/solido/volumen-alto.svg"),
    ),
    (
        "volumen-bajo",
        "solido/volumen-bajo",
        include_bytes!("../assets/iconos/solido/volumen-bajo.svg"),
    ),
    (
        "volumen-medio",
        "solido/volumen-medio",
        include_bytes!("../assets/iconos/solido/volumen-medio.svg"),
    ),
    (
        "volumen-silencio",
        "solido/volumen-silencio",
        include_bytes!("../assets/iconos/solido/volumen-silencio.svg"),
    ),
    (
        "wifi",
        "solido/wifi",
        include_bytes!("../assets/iconos/solido/wifi.svg"),
    ),
];

/// Un icono propio por su nombre. `None` si no existe, que solo puede pasar por
/// una errata al escribirlo.
///
/// `escritorio-N` no está en la tabla: se dibuja al vuelo con el número dentro,
/// que es lo único que lo distingue del anterior. Ver [`escritorio`].
/// Va por la misma [`CACHE`] que [`cargar`], y no es una optimización de
/// adorno: `Handle::from_memory` **estrena identificador en cada llamada**, así
/// que iced no puede reutilizar nada y resvg vuelve a rasterizar el dibujo en
/// cada fotograma. Medido en la isla de actividades, con sus ocho pictogramas:
/// un frame del reproductor pasa de 9 ms a 2, y con la cola abierta de 19 a 5.
pub fn propio(nombre: &str) -> Option<Icono> {
    con_cache(nombre, || propio_sin_cache(nombre))
}

/// La variante maciza de [`propio`], o el de siempre si no la tiene. Ver
/// [`SOLIDOS`].
pub fn propio_solido(nombre: &str) -> Option<Icono> {
    match SOLIDOS.iter().find(|(n, _, _)| *n == nombre) {
        Some((_, clave, bytes)) => {
            con_cache(clave, || Some(Icono::Svg(svg::Handle::from_memory(*bytes))))
        }
        None => propio(nombre),
    }
}

/// Resuelve únicamente el catálogo de BookOS sin tocar la caché compartida.
/// `cargar` lo necesita para poder completar después la búsqueda en los temas
/// del sistema antes de decidir si guarda un `None` definitivo.
fn propio_sin_cache(nombre: &str) -> Option<Icono> {
    if let Some(n) = nombre.strip_prefix("escritorio-") {
        return n.parse().ok().map(escritorio);
    }
    PROPIOS
        .iter()
        .find(|(n, _)| *n == nombre)
        .map(|(_, bytes)| Icono::Svg(svg::Handle::from_memory(*bytes)))
}

/// Lo que ya hubiera guardado con esa clave, o lo que devuelva `construir`.
fn con_cache(clave: &str, construir: impl FnOnce() -> Option<Icono>) -> Option<Icono> {
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(icono) = cache.get(clave) {
        return icono.clone();
    }
    let icono = construir();
    cache.insert(clave.to_string(), icono.clone());
    icono
}

/// El monitor de heroicons (`16/solid/computer-desktop`) con el número del
/// escritorio dentro de la pantalla.
///
/// El número va **dentro del hueco** que el propio icono deja: su path usa
/// `fill-rule="evenodd"` y el segundo subpath vacía la pantalla, así que ahí no
/// hay relleno y el dígito se lee sobre el fondo de la cápsula.
///
/// Se construye en memoria en vez de tener cinco ficheros —uno por escritorio—
/// porque lo único que cambia entre ellos es un dígito, y cinco copias del
/// mismo monitor se separan a la primera corrección del dibujo.
pub fn escritorio(numero: u32) -> Icono {
    let digito = digito(numero);
    desde_svg(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#ffffff">
  <path fill-rule="evenodd" d="M2 4.25A2.25 2.25 0 0 1 4.25 2h7.5A2.25 2.25 0 0 1 14 4.25v5.5A2.25 2.25 0 0 1 11.75 12h-1.312c.1.128.21.248.328.36a.75.75 0 0 1 .234.545v.345a.75.75 0 0 1-.75.75h-4.5a.75.75 0 0 1-.75-.75v-.345a.75.75 0 0 1 .234-.545c.118-.111.228-.232.328-.36H4.25A2.25 2.25 0 0 1 2 9.75v-5.5Zm2.25-.75a.75.75 0 0 0-.75.75v4.5c0 .414.336.75.75.75h7.5a.75.75 0 0 0 .75-.75v-4.5a.75.75 0 0 0-.75-.75h-7.5Z" clip-rule="evenodd"/>
  <path d="{digito}" fill="none" stroke="#ffffff" stroke-width="0.9"
        stroke-linecap="round" stroke-linejoin="round"/>
</svg>"##
    ))
}

/// El trazo de un dígito, centrado en el hueco de la pantalla del monitor.
///
/// Dibujado y no escrito con `<text>`: resvg necesita una base de fuentes para
/// rasterizar texto y la que usa iced por dentro no la tiene, así que el
/// `<text>` salía **vacío** —comprobado pintando el aviso a escala 8, el
/// monitor aparecía y el número no—. Con trazo, además, el grosor no depende
/// de qué fuente haya instalada.
///
/// El hueco va de (3,5, 3,5) a (12,5, 9,5) en el viewBox de 16; los dígitos
/// ocupan la caja (6,6 – 9,4) × (4,4 – 8,5), o sea centrados y con aire por los
/// cuatro lados. Solo hay hasta el 5 porque `escritorios::MAXIMO` son cinco: un
/// número mayor deja el monitor sin dígito en vez de dibujar cualquier cosa.
fn digito(numero: u32) -> &'static str {
    match numero {
        1 => "M7.38 5.36 8.13 4.65 8.13 8.26",
        2 => "M6.9 5.49C6.9 4.3 9.19 4.3 9.19 5.88 9.19 6.94 7.21 7.38 6.86 8.26L9.23 8.26",
        3 => {
            "M6.94 5.14C7.43 4.34 9.14 4.56 9.14 5.62 9.14 6.24 8.53 6.46 8.09 6.46 8.62 6.46 9.28 6.72 9.28 7.38 9.28 8.57 7.3 8.66 6.9 7.82"
        }
        4 => "M8.66 8.26 8.66 4.65 6.77 7.25 9.23 7.25",
        5 => {
            "M9.06 4.74 7.16 4.74 7.03 6.37C7.65 6.02 9.28 6.19 9.28 7.29 9.28 8.52 7.43 8.66 6.94 7.86"
        }
        _ => "",
    }
}

/// Los iconos ya resueltos, por nombre.
///
/// **No es solo por ahorrarse la búsqueda.** Un `Handle` de iced lleva dentro un
/// identificador, y el rasterizado se cachea por ese identificador: devolver un
/// handle nuevo para el mismo icono obliga a iced a rasterizarlo otra vez.
/// Medido en release, abrir el conmutador con cinco iconos de 64 px costaba
/// **46,6 ms cada vez** —no solo la primera—, contra 0,32 ms de repintarlo.
/// Devolviendo el mismo handle, esos 46 ms se pagan una vez por sesión.
///
/// El `None` también se guarda: un icono que no está tampoco lo va a estar
/// dentro de un segundo, y buscarlo son unos cientos de `stat`. Pero sí puede
/// estar tras instalar la app: [`olvidar_ausentes`] los tira al releer los
/// `.desktop`.
static CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Option<Icono>>>,
> = std::sync::OnceLock::new();

/// Busca el icono por nombre y lo carga. `None` si no aparece o está roto.
///
/// Primero los propios: un icono de BookOS no debe cambiar de dibujo porque el
/// usuario instale otro tema.
pub fn cargar(nombre: &str) -> Option<Icono> {
    if es_dinamico(nombre) {
        return cargar_dinamico(nombre);
    }
    if es_temporal(nombre) {
        return cargar_sin_cache(nombre);
    }
    let cache = CACHE.get_or_init(Default::default);
    {
        // No se conserva el bloqueo mientras se resuelve el icono. La ruta de
        // resolución consulta primero `propio()`, que comparte esta caché; si
        // entrase con el mutex cogido intentaría bloquearlo por segunda vez y
        // congelaría el compositor durante su primer arranque.
        let mapa = match cache.lock() {
            Ok(mapa) => mapa,
            Err(envenenado) => envenenado.into_inner(),
        };
        if let Some(guardado) = mapa.get(nombre) {
            return guardado.clone();
        }
    }

    let icono = cargar_sin_cache(nombre);

    // Otro hilo puede haber resuelto la misma clave mientras no teníamos el
    // mutex. En ese caso se conserva su Handle para que iced siga viendo un
    // único identificador de textura por icono.
    let mut mapa = match cache.lock() {
        Ok(mapa) => mapa,
        Err(envenenado) => envenenado.into_inner(),
    };
    mapa.entry(nombre.to_string())
        .or_insert_with(|| icono.clone())
        .clone()
}

/// Iconos que su aplicación reescribe en disco: el reloj mueve las agujas del
/// SVG del tema cada minuto (`reloj-dinamico` del pack de iconos) y el
/// calendario deja el PNG con el día de hoy en hicolor. Con la caché por nombre
/// se quedaban con el dibujo del arranque de la sesión.
const DINAMICOS: &[&str] = &["bookos-clock", "bookos-calendar"];

pub fn es_dinamico(nombre: &str) -> bool {
    DINAMICOS.contains(&nombre)
}

/// Cuándo se reescribió el icono dinámico; `None` si no lo es o no está.
/// El dock lo compara para no releer un fichero que no ha cambiado.
pub fn modificado_si_dinamico(nombre: &str) -> Option<std::time::SystemTime> {
    if !es_dinamico(nombre) {
        return None;
    }
    std::fs::metadata(resolve_icon(nombre)?)
        .ok()?
        .modified()
        .ok()
}

/// Rutas donde las aplicaciones dejan imágenes de usar y tirar: los avatares
/// y carátulas que acompañan a un aviso (`image-path`) suelen tener un nombre
/// distinto en cada uno. Por la caché, que no olvida nada en toda la sesión,
/// cada aviso dejaba su mapa de bits —hasta 256 KB a [`LADO_RASTER`]— para
/// siempre. El aviso ya se queda con su `Icono`, así que no hace falta.
fn es_temporal(nombre: &str) -> bool {
    ["/tmp/", "/var/tmp/", "/run/"]
        .iter()
        .any(|dir| nombre.starts_with(dir))
}

/// Sin caché, y el SVG desde sus bytes en vez de desde la ruta: el
/// identificador del `Handle` sale de lo que se le da, así que por ruta iced
/// seguiría sirviendo el rasterizado de la hora vieja. Por contenido solo se
/// rasteriza de nuevo cuando el dibujo cambia de verdad.
fn cargar_dinamico(nombre: &str) -> Option<Icono> {
    let ruta = resolve_icon(nombre)?;
    if ruta.extension().is_some_and(|e| e == "svg") {
        let bytes = std::fs::read(&ruta).ok()?;
        return Some(Icono::Svg(svg::Handle::from_memory(bytes)));
    }
    load_icon(&ruta)
}

fn cargar_sin_cache(nombre: &str) -> Option<Icono> {
    if let Some(icono) = propio_sin_cache(nombre) {
        return Some(icono);
    }
    resolve_icon(nombre).and_then(|ruta| load_icon(&ruta))
}

/// Dónde mirar, en orden. Primero el tema propio de BookOS si existe, luego los
/// del sistema.
fn icon_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/icons"));
    }
    dirs.push(PathBuf::from("/usr/share/icons"));
    dirs.push(PathBuf::from("/usr/share/pixmaps"));
    dirs
}

/// `breeze-dark` va **antes** que `breeze`: los iconos monocromos de estado de
/// Breeze llevan el color dentro (`color:#232629`, casi negro, para fondos
/// claros) y sobre el panel negro no se veía absolutamente nada. La variante
/// oscura trae los mismos dibujos con `#fcfcfc`.
const TEMAS: &[&str] = &[
    "BookOS-Light",
    "BookOS",
    "breeze-dark",
    "breeze",
    "hicolor",
    "Adwaita",
];
/// El tema de iconos que el usuario eligió en Ajustes → Apariencia → Iconos
/// (Claro, Oscuro o Tintados), que Ajustes deja en `icon-theme` de GSettings.
///
/// `TEMAS` no basta: solo conoce `BookOS-Light` y `BookOS`, así que el pack
/// oscuro y `BookOS-Tinted`/`BookOS-Tinted-B` (Ajustes alterna los dos nombres
/// para que GTK vea cada cambio de paleta) se aplicaban en las apps y no en el
/// dock. Se lee por `gsettings` porque es donde Ajustes lo escribe; el nombre
/// se descarta si no es un directorio simple, ya que acaba en una ruta.
pub fn tema_elegido() -> Option<String> {
    let salida = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let texto = String::from_utf8(salida.stdout).ok()?;
    let nombre = texto.trim().trim_matches('\'');
    (!nombre.is_empty() && !nombre.contains(['/', '.'])).then(|| nombre.to_string())
}

/// Las categorías de freedesktop que usa el escritorio. `status` y
/// `preferences` hacen falta para el panel: los iconos de aplicación viven en
/// `apps`, pero los de batería y red están en `status`.
///
/// `mimetypes` y `places` son las de los iconos del escritorio: un PNG resuelve
/// a `image-x-generic`, que vive en `mimetypes`, y una carpeta a `folder`, que
/// vive en `places`. Sin ellas la rejilla salía con los nombres y sin ningún
/// dibujo — se vio en la previsualización.
const CATEGORIAS: &[&str] = &[
    "status",
    "apps",
    "actions",
    "devices",
    "preferences",
    "categories",
    "mimetypes",
    "places",
];
/// El tamaño que anuncia el nombre de un directorio de tema, para ordenar de
/// mayor a menor: es preferible reducir un icono grande que ampliar uno de 16
/// px y que se vea pastoso. `scalable` y `symbolic` ganan a cualquier píxel.
///
/// Aquí había una lista de tamaños escrita a mano y **se quedaba corta**:
/// faltaban `32x32` y `512x512`, que es donde hicolor guarda los iconos de
/// Firefox y de bookos-settings, así que el dock los dibujaba con la inicial.
/// Enumerar lo que hay en disco no puede quedarse corto.
fn tamano_de(nombre: &str) -> Option<u32> {
    if nombre == "scalable" || nombre == "symbolic" {
        return Some(u32::MAX);
    }
    // `32x32`, `32`, `32x32@2` — basta el primer número.
    nombre.split(['x', '@']).next()?.parse().ok()
}

/// Los directorios donde de verdad hay iconos, en orden de preferencia.
///
/// Se calcula **una vez** por proceso. La búsqueda de freedesktop es un
/// producto de bases por temas por tamaños por categorías, y recorrerlo entero
/// por cada icono salían unas 4.700 llamadas a `stat`; con un icono que no
/// existe se pagaban todas. Como la mayoría de esas combinaciones no existen en
/// disco —aquí sobreviven unas pocas decenas—, filtrarlas una vez convierte
/// cada búsqueda posterior en un puñado de comprobaciones.
///
/// Medido en esta máquina: buscar treinta iconos pasó de 31 ms a menos de 2.
fn directorios() -> std::sync::Arc<Vec<PathBuf>> {
    con_directorios(|| {
        let elegido = tema_elegido();
        let mut dirs = Vec::new();
        for base in icon_dirs() {
            if !base.is_dir() {
                continue;
            }
            // Los planos como /usr/share/pixmaps van tal cual, sin tema.
            dirs.push(base.clone());
            for tema in elegido
                .iter()
                .map(String::as_str)
                .chain(TEMAS.iter().copied())
            {
                let raiz = base.join(tema);
                let Ok(nivel1) = std::fs::read_dir(&raiz) else {
                    continue;
                };
                // Los temas mezclan tema/tamaño/categoría y
                // tema/categoría/tamaño, así que se mira quién de los dos es
                // una categoría conocida y el otro da el tamaño.
                let mut del_tema: Vec<(u32, PathBuf)> = Vec::new();
                for a in nivel1.flatten() {
                    let Ok(nombre_a) = a.file_name().into_string() else {
                        continue;
                    };
                    if let Some(tam) = tamano_de(&nombre_a) {
                        // tema/tamaño/categoría: se comprueban las seis
                        // categorías con `stat`. Enumerar el directorio en su
                        // lugar costaba 43 ms de arranque —hicolor y Adwaita
                        // tienen cientos de carpetas con miles de ficheros— y
                        // el panel se pinta en el primer frame.
                        for sub in CATEGORIAS {
                            let d = a.path().join(sub);
                            if d.is_dir() {
                                del_tema.push((tam, d));
                            }
                        }
                    } else if CATEGORIAS.contains(&nombre_a.as_str()) {
                        // tema/categoría/tamaño: aquí sí hay que enumerar, pero
                        // solo son seis directorios por tema y contienen
                        // carpetas, no iconos.
                        let Ok(nivel2) = std::fs::read_dir(a.path()) else {
                            continue;
                        };
                        for b in nivel2.flatten() {
                            let Ok(nombre_b) = b.file_name().into_string() else {
                                continue;
                            };
                            if let Some(tam) = tamano_de(&nombre_b)
                                && b.path().is_dir()
                            {
                                del_tema.push((tam, b.path()));
                            }
                        }
                    }
                }
                // De mayor a menor dentro del tema; los temas conservan entre
                // sí el orden de preferencia de `TEMAS`.
                del_tema.sort_by_key(|d| std::cmp::Reverse(d.0));
                dirs.extend(del_tema.into_iter().map(|(_, d)| d));
            }
        }
        tracing::debug!(n = dirs.len(), "directorios de iconos");
        dirs
    })
}

fn con_directorios(construir: impl FnOnce() -> Vec<PathBuf>) -> std::sync::Arc<Vec<PathBuf>> {
    let mut guardado = DIRS.lock().unwrap_or_else(|e| e.into_inner());
    guardado
        .get_or_insert_with(|| std::sync::Arc::new(construir()))
        .clone()
}

/// Los directorios del tema elegido, que dependen de `icon-theme` y por eso
/// se pueden tirar: Ajustes cambia el tema sin reiniciar la sesión.
static DIRS: std::sync::Mutex<Option<std::sync::Arc<Vec<PathBuf>>>> = std::sync::Mutex::new(None);

/// Olvida el tema de iconos leído y todos los iconos ya cargados, para que la
/// próxima búsqueda mire el `icon-theme` de ahora. Sin esto el dock se quedaba
/// con los iconos del arranque: `directorios()` y `CACHE` valen para toda la
/// sesión y Ajustes alterna `BookOS-Tinted`/`-B` justo para que se note el cambio.
/// Olvida los iconos que se buscaron y no estaban, y conserva los encontrados.
///
/// Una app abierta antes de instalar su paquete (o instalada desde la tienda
/// con el dock ya pidiendo su icono) dejaba guardado el `None` toda la sesión,
/// y el launchpad la enseñaba con la inicial aunque el icono ya estuviera en
/// disco. Se llama donde se releen los `.desktop`, que es cuando una
/// instalación se hace visible.
pub fn olvidar_ausentes() {
    if let Some(cache) = CACHE.get() {
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, icono| icono.is_some());
    }
}

pub fn recargar_tema() {
    *DIRS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if let Some(cache) = CACHE.get() {
        cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

pub fn resolve_icon(name: &str) -> Option<PathBuf> {
    // Un nombre puede venir ya como ruta absoluta desde un `.desktop`.
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    for dir in directorios().iter() {
        for ext in ["svg", "png", "xpm"] {
            let p = dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Lado máximo, en píxeles, al que se guarda un icono de mapa de bits.
///
/// El icono más grande que se dibuja es el del launchpad, unos 78 px lógicos:
/// a escala 2 son 156, y 256 deja margen para una pantalla a escala 3.
const LADO_RASTER: u32 = 256;

/// Carga un icono desde disco.
///
/// Los SVG se los queda iced (los rasteriza resvg al tamaño que toque). Los
/// mapas de bits se decodifican **aquí**: si el fichero está corrupto o en un
/// formato que no soportamos, devolvemos `None` y el item cae en la baldosa con
/// la inicial. Dejárselo a iced significaría un panic, y un panic del shell
/// apaga el panel y el dock enteros — un icono roto no puede costar tanto.
fn load_icon(path: &Path) -> Option<Icono> {
    if path.extension().is_some_and(|e| e == "svg") {
        return Some(Icono::Svg(svg::Handle::from_path(path)));
    }
    match image::ImageReader::open(path).ok()?.decode() {
        Ok(img) => {
            // La caché los guarda toda la sesión, y hay aplicaciones que traen
            // un PNG de 1024 px: medido, uno solo eran 4,2 MB de los 33 que el
            // compositor tenía reservados en reposo.
            let img = if img.width().max(img.height()) > LADO_RASTER {
                img.resize(
                    LADO_RASTER,
                    LADO_RASTER,
                    image::imageops::FilterType::Lanczos3,
                )
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            Some(Icono::Raster(iced_image::Handle::from_rgba(
                w,
                h,
                rgba.into_raw(),
            )))
        }
        Err(err) => {
            tracing::warn!(?path, "no se pudo decodificar el icono: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada escritorio tiene su icono con su número **dibujado**, no escrito:
    /// un `<text>` aquí sale vacío porque resvg no tiene fuentes cargadas, y el
    /// aviso enseñaría un monitor en blanco. Este test es lo que impide volver
    /// a caer en ello.
    #[test]
    fn el_icono_de_escritorio_lleva_su_numero() {
        for n in 1..=crate::config::MAXIMO_ESCRITORIOS {
            let trazo = digito(n as u32);
            assert!(!trazo.is_empty(), "el escritorio {n} se queda sin número");
            assert!(trazo.starts_with('M'), "el dígito {n} no empieza con un M");
        }
        assert!(propio("escritorio-3").is_some());
        // Y un número fuera de la cuenta deja el monitor sin dígito en vez de
        // dibujar cualquier cosa.
        assert!(digito(0).is_empty());
        assert!(digito(9).is_empty());
        assert!(propio("escritorio-x").is_none());
    }

    /// Los nombres del dock por defecto tienen que resolverse en una máquina
    /// con Breeze y hicolor instalados. Es el test que habría cazado que
    /// faltaba `32x32` en la lista de tamaños: Firefox salía sin icono.
    #[test]
    fn los_iconos_del_dock_se_encuentran() {
        let t0 = std::time::Instant::now();
        let n = directorios().len();
        let indexado = t0.elapsed();
        let t1 = std::time::Instant::now();
        for nombre in [
            "utilities-terminal",
            "bookos-explorer",
            "firefox",
            "bookos-notepad",
        ] {
            assert!(resolve_icon(nombre).is_some(), "sin icono para {nombre}");
        }
        println!(
            "{n} directorios en {indexado:?}; 4 búsquedas en {:?}",
            t1.elapsed()
        );
    }

    /// `cargar` consulta primero los iconos propios y ambos caminos comparten
    /// caché. Esta llamada a un icono externo reproducía el interbloqueo que
    /// dejaba BookOS detenido antes de su primer fotograma.
    #[test]
    fn olvidar_ausentes_solo_tira_los_que_no_estaban() {
        let cache = CACHE.get_or_init(Default::default);
        cache.lock().expect("caché sin envenenar").extend([
            ("bookos-prueba-ausente".to_string(), None),
            (
                "bookos-prueba-presente".to_string(),
                Some(desde_svg("<svg/>")),
            ),
        ]);
        olvidar_ausentes();
        let mapa = cache.lock().expect("caché sin envenenar");
        assert!(!mapa.contains_key("bookos-prueba-ausente"));
        assert!(mapa.contains_key("bookos-prueba-presente"));
    }

    #[test]
    fn cargar_un_icono_externo_no_reentra_con_el_mutex_cogido() {
        assert!(cargar("network-disconnect").is_some());
        // La segunda consulta recorre el camino caliente de la caché.
        assert!(cargar("network-disconnect").is_some());
    }
}
