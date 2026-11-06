//! Gestión de pantallas: modelo, validación, persistencia y aplicación.
//!
//! Es el reemplazo de KScreen para las sesiones BookOS. La división es
//! deliberada y tiene tres piezas que no se tocan entre sí:
//!
//! 1. **Este fichero** define el modelo, valida una configuración y la guarda.
//!    No conoce DRM, ni EGL, ni el renderer: todo lo que hay aquí se puede
//!    probar con `cargo test` sin una GPU delante.
//! 2. **El backend** ([`crate::backend::udev`], [`crate::backend::winit`])
//!    instala un [`Aplicador`]: el único trozo de código que toca KMS. Se llama
//!    siempre desde el hilo dueño del compositor.
//! 3. **[`crate::ajustes`]** publica la interfaz D-Bus. El hilo de zbus no ve
//!    nunca ni el renderer ni el backend: lee un `Mutex` con el último censo y
//!    empuja las peticiones por el canal de calloop, como ya hacía la recarga
//!    de la pantalla de bloqueo.
//!
//! **Por qué el identificador no es el nombre del conector.** `eDP-1` o
//! `DP-3` los reparte el kernel por orden de sondeo: enchufar el monitor en el
//! otro puerto del dock cambia el nombre y la configuración guardada se
//! aplicaría a la pantalla equivocada. El identificador sale del EDID
//! —fabricante, modelo y número de serie—, que viaja con el monitor. Solo
//! cuando el EDID no dice nada útil se cae al nombre del conector.
//!
//! **Restauración segura.** Una configuración que deja todas las salidas
//! apagadas deja el portátil inutilizable sin forma de arreglarlo desde la
//! propia sesión. Ni [`validar`] la acepta ni [`cargar`] la devuelve, así que
//! un fichero editado a mano con todo a `activa=no` se ignora entero en vez de
//! dejar la pantalla negra.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// Versión del contrato D-Bus. Sube cuando cambie la **forma** de los tipos,
/// no cuando se añada una capacidad: las capacidades se anuncian por su cuenta
/// en `GetCapabilities`, que es un diccionario justamente para poder crecer
/// sin romper a quien ya habla con nosotros.
pub const VERSION_CONTRATO: u32 = 1;

/// Escalas que se ofrecen en la interfaz. Un cuarto es el paso que reconocen
/// todos los toolkits con `wp_fractional_scale`, y es también el que usa la
/// heurística de [`crate::backend::escala_sugerida`], así que lo que sugiere el
/// compositor solo puede caer en esta lista.
pub const ESCALAS: [f64; 9] = [1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 2.75, 3.0];

/// Fuera de este rango una escala no es "poco cómoda", es inservible: por
/// debajo el panel no se lee y por encima no cabe una ventana en la pantalla.
pub const ESCALA_MIN: f64 = 0.5;
pub const ESCALA_MAX: f64 = 4.0;
/// La escala se admite en pasos de 0,05 aunque la lista ofrecida sea de
/// cuartos: quien quiera 1,15 desde el fichero puede, pero 1,7333… no, porque
/// el redondeo del tamaño lógico deja de ser reproducible.
const PASO_ESCALA: f64 = 0.05;

/// Ninguna pantalla real se coloca a 100.000 px del origen. El límite existe
/// para que un valor absurdo no mande las ventanas a un sitio del que no se
/// pueden rescatar con el ratón.
const POSICION_MAX: i32 = 32_768;

// ── Modelo ───────────────────────────────────────────────────────────────

/// Un modo de vídeo. El refresco va en **milihercios**, que es la unidad de
/// `wl_output` y la que permite distinguir 59,94 de 60,00 sin coma flotante.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Modo {
    pub ancho: u32,
    pub alto: u32,
    pub refresco_mhz: u32,
    /// El que el monitor anuncia como nativo.
    pub preferido: bool,
    pub actual: bool,
}

/// Una salida tal y como la ve quien configura: lo que hay, no lo que se pide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Salida {
    /// Identificador estable, el que se persiste. Ver la nota del módulo.
    pub id: String,
    /// Nombre del conector del kernel: `eDP-1`, `DP-2`, `winit`.
    pub conector: String,
    pub fabricante: String,
    pub modelo: String,
    pub serie: String,
    /// Tamaño físico del panel en milímetros. `0` si el EDID no lo dice.
    pub mm_ancho: u32,
    pub mm_alto: u32,
    pub activa: bool,
    pub modos: Vec<Modo>,
    pub escala: f64,
    /// Las escalas que se ofrecen. Va en el censo y no cableada en la interfaz
    /// para que Settings no tenga que saber nada de la política del compositor.
    pub escalas: Vec<f64>,
    pub x: i32,
    pub y: i32,
    /// `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180`,
    /// `flipped-270`.
    pub transformacion: String,
    pub vrr_capaz: bool,
    pub vrr: bool,
    pub principal: bool,
    /// Tamaño en píxeles **lógicos**, ya con la escala y la rotación aplicadas.
    /// Se manda calculado porque es lo que decide si dos pantallas se solapan,
    /// y repetir ese cálculo en el cliente es repetir el redondeo.
    pub logico_ancho: i32,
    pub logico_alto: i32,
}

/// Lo que se pide para una salida. Es lo que viaja en `ApplyOutputConfig` y lo
/// que se persiste, con los mismos campos: así lo guardado y lo aplicado no
/// pueden divergir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Peticion {
    pub id: String,
    pub activa: bool,
    pub ancho: u32,
    pub alto: u32,
    pub refresco_mhz: u32,
    pub escala: f64,
    pub x: i32,
    pub y: i32,
    pub transformacion: String,
    pub vrr: bool,
    pub principal: bool,
}

/// Lo que devuelve el backend después de tocar el hardware.
pub struct Aplicado {
    /// El censo recién releído: es la verdad, no lo que se pidió.
    pub salidas: Vec<Salida>,
    /// La salida de Smithay de cada pantalla encendida y dónde va en el
    /// escritorio, en píxeles lógicos.
    pub mapa: Vec<(
        smithay::output::Output,
        smithay::utils::Point<i32, smithay::utils::Logical>,
    )>,
    /// Índice dentro de `mapa` de la que lleva el panel y el dock.
    pub principal: usize,
}

/// El único trozo de código que toca KMS. Lo instala el backend al arrancar y
/// se invoca **solo** desde el hilo del compositor.
///
/// Recibe la configuración ya validada y devuelve el censo nuevo, o el motivo
/// por el que el hardware la rechazó. Que devuelva `Result` y no `()` es lo que
/// permite que `ApplyOutputConfig` no mienta: si el modeset falla, Settings se
/// entera.
pub type Aplicador = Box<dyn Fn(&[Peticion]) -> Result<Aplicado, String>>;

/// Vuelve a mirar qué hay enchufado, sin cambiar nada. Lo instala el backend
/// junto al [`Aplicador`] y se usa al arrancar y cuando el kernel avisa de un
/// cambio de conector.
pub type Censador = Box<dyn Fn() -> Vec<Salida>>;

/// Lo que el hilo de D-Bus puede leer sin sincronizarse con el compositor.
///
/// Es un censo, no un puntero al backend: se rellena desde el hilo del
/// compositor cada vez que las salidas cambian, y zbus solo lo copia. Sin esto
/// habría que despertar al compositor para contestar a un `GetOutputs`, que es
/// justo lo que el proyecto evita.
pub struct Compartido {
    salidas: Mutex<Vec<Salida>>,
    /// `udev` o `winit`. Cambia lo que se puede hacer, y Settings lo enseña.
    pub backend: Mutex<&'static str>,
}

impl Default for Compartido {
    // 0.1.0
    fn default() -> Self {
        Self {
            salidas: Mutex::new(Vec::new()),
            backend: Mutex::new("ninguno"),
        }
    }
}

impl Compartido {
    // 0.1.0
    pub fn salidas(&self) -> Vec<Salida> {
        // Un `Mutex` envenenado significa que el hilo del compositor entró en
        // pánico mientras publicaba. La sesión ya está perdida; devolver el
        // censo de todas formas es mejor que arrastrar el pánico al bus.
        match self.salidas.lock() {
            Ok(guard) => guard.clone(),
            Err(err) => err.into_inner().clone(),
        }
    }

    // 0.1.0
    pub fn publicar(&self, salidas: Vec<Salida>) {
        match self.salidas.lock() {
            Ok(mut guard) => *guard = salidas,
            Err(err) => *err.into_inner() = salidas,
        }
    }

    /// Lo que este backend sabe hacer, para `GetCapabilities`. Diccionario y no
    /// estructura para poder añadir capacidades sin romper la firma D-Bus.
    // 0.1.0
    pub fn capacidades(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
        use zbus::zvariant::Value;
        let backend = *self.backend.lock().unwrap_or_else(|e| e.into_inner());
        let udev = backend == "udev";
        let mut m = std::collections::HashMap::new();
        let mut poner = |k: &str, v: Value<'static>| {
            if let Ok(v) = zbus::zvariant::OwnedValue::try_from(v) {
                m.insert(k.to_string(), v);
            }
        };
        poner("version", Value::from(VERSION_CONTRATO));
        poner("backend", Value::from(backend.to_string()));
        // Todo lo que sigue se aplica sin reiniciar la sesión.
        poner("live_apply", Value::from(true));
        // `teclas.conf`: GetKeyRemaps, ApplyKeyRemaps y CaptureKey.
        poner("key_remap", Value::from(true));
        // Hay sensor de luz: Settings puede ofrecer `brillo_automatico_*`.
        poner(
            "ambient_light",
            Value::from(bookos_shell::retroiluminacion::hay_sensor_luz()),
        );
        poner("fractional_scale", Value::from(true));
        poner("per_output_scale", Value::from(true));
        // El modelo entiende un escritorio lógico bidimensional aunque el
        // backend anidado solo tenga una salida y el DRM aún anuncie aparte si
        // puede encender varias. Settings puede ofrecer el editor de posiciones
        // sin confundir esta capacidad con `multi_output`.
        poner("virtual_layout", Value::from(true));
        poner("layout_negative_coordinates", Value::from(true));
        poner("position", Value::from(udev));
        poner("rotation", Value::from(udev));
        poner("mode", Value::from(udev));
        poner("refresh", Value::from(udev));
        poner("vrr", Value::from(udev));
        poner("multi_output", Value::from(udev));
        poner("hotplug", Value::from(udev));
        poner("per_output_refresh", Value::from(udev));
        poner("mixed_scale_spanning", Value::from(udev));
        poner("presentation_time", Value::from(udev));
        // Hoy ambas barras viajan juntas con la salida principal. El contrato
        // lo anuncia para que Settings pueda explicar qué monitor las lleva.
        poner("shell_follows_primary", Value::from(true));
        poner("independent_panel_dock_outputs", Value::from(false));
        poner("primary", Value::from(true));
        // Puntos de extensión declarados a propósito: la interfaz ya los
        // anuncia como ausentes para que Settings no tenga que adivinar.
        poner("hdr", Value::from(false));
        poner("icc", Value::from(false));
        // La curva de gamma solo existe con KMS: anidado, la pantalla es del
        // compositor anfitrión. Ver `crate::luz_nocturna`.
        poner("night_light", Value::from(udev));
        poner("escalas", Value::from(ESCALAS.to_vec()));
        m
    }
}

/// Lo que el compositor guarda sobre las pantallas.
#[derive(Default)]
pub struct Estado {
    pub compartido: Arc<Compartido>,
    /// La última configuración aplicada con éxito. Es a lo que se vuelve
    /// cuando una nueva no cuela.
    pub ultima: Vec<Peticion>,
}

// ── Validación ───────────────────────────────────────────────────────────

/// ¿Es un nombre de transformación de los que entiende el compositor?
// 0.1.0
pub fn transformacion_valida(t: &str) -> bool {
    transformacion(t).is_some()
}

/// El `Transform` de Smithay que corresponde al nombre, si existe.
// 0.1.0
pub fn transformacion(t: &str) -> Option<smithay::utils::Transform> {
    use smithay::utils::Transform::*;
    Some(match t {
        "normal" => Normal,
        "90" => _90,
        "180" => _180,
        "270" => _270,
        "flipped" => Flipped,
        "flipped-90" => Flipped90,
        "flipped-180" => Flipped180,
        "flipped-270" => Flipped270,
        _ => return None,
    })
}

/// El nombre de un `Transform`, para el camino de vuelta.
// 0.1.0
pub fn nombre_transformacion(t: smithay::utils::Transform) -> &'static str {
    use smithay::utils::Transform as T;
    match t {
        T::Normal => "normal",
        T::_90 => "90",
        T::_180 => "180",
        T::_270 => "270",
        T::Flipped => "flipped",
        T::Flipped90 => "flipped-90",
        T::Flipped180 => "flipped-180",
        T::Flipped270 => "flipped-270",
    }
}

/// ¿La rotación intercambia ancho y alto?
// 0.1.0
fn gira_un_cuarto(t: &str) -> bool {
    matches!(t, "90" | "270" | "flipped-90" | "flipped-270")
}

/// Tamaño lógico de una salida ya con escala y rotación aplicadas.
///
/// Se redondea hacia arriba y no al entero más próximo: a la baja, una pantalla
/// de 2880 px a 1,75 daría 1645 lógicos y la última fila de píxeles físicos no
/// tendría a nadie que la pintara.
/// El nombre que ve el usuario. El del conector —`HDMI-A-1`, `eDP-1`— es el
/// del kernel y no dice qué monitor es; el modelo del EDID sí. El panel
/// interno se llama por lo que es, porque su EDID suele traer un código de
/// pieza (`ATNA40YK`) que nadie reconoce.
// 0.1.0
pub fn nombre_visible(conector: &str, modelo: &str) -> String {
    let interno = ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|prefijo| conector.starts_with(prefijo));
    if interno {
        "Pantalla integrada".into()
    } else if modelo.is_empty() || modelo == "KMS" {
        // "KMS" es el relleno de `udev.rs` cuando no hay EDID.
        conector.into()
    } else {
        modelo.into()
    }
}

// 0.1.0
pub fn tamano_logico(ancho: u32, alto: u32, escala: f64, transformacion: &str) -> (i32, i32) {
    let (w, h) = if gira_un_cuarto(transformacion) {
        (alto, ancho)
    } else {
        (ancho, alto)
    };
    (
        (w as f64 / escala).ceil() as i32,
        (h as f64 / escala).ceil() as i32,
    )
}

/// ¿Es una escala que el compositor puede aplicar?
///
/// No basta con `> 0`: un `NaN` pasa cualquier comparación que se le ponga
/// delante y acabaría dividiendo el tamaño de la pantalla, que es como se
/// consigue una salida de 0×0 y un compositor que no vuelve a arrancar.
// 0.1.0
pub fn escala_valida(escala: f64) -> bool {
    if !escala.is_finite() || escala <= 0.0 {
        return false;
    }
    if escala < ESCALA_MIN - f64::EPSILON || escala > ESCALA_MAX + f64::EPSILON {
        return false;
    }
    // Tolerancia generosa: el valor llega de un `f64` que ha pasado por JSON y
    // por D-Bus, y 1,75 puede volver como 1,7499999999999998.
    let pasos = escala / PASO_ESCALA;
    (pasos - pasos.round()).abs() < 1e-6
}

/// Comprueba que una configuración se puede aplicar sin dejar la sesión
/// inservible. Es pura a propósito: es la parte que se puede probar sin GPU.
// 0.1.0
pub fn validar(actual: &[Salida], peticion: &[Peticion]) -> Result<(), String> {
    if peticion.is_empty() {
        return Err("la configuración no menciona ninguna salida".into());
    }

    let mut vistos: Vec<&str> = Vec::with_capacity(peticion.len());
    for p in peticion {
        if vistos.contains(&p.id.as_str()) {
            return Err(format!("la salida «{}» aparece dos veces", p.id));
        }
        vistos.push(&p.id);
        if !actual.iter().any(|s| s.id == p.id) {
            return Err(format!("no hay ninguna salida «{}»", p.id));
        }
    }

    let activas: Vec<&Peticion> = peticion.iter().filter(|p| p.activa).collect();
    if activas.is_empty() {
        return Err(
            "una configuración sin ninguna pantalla encendida dejaría la sesión a ciegas".into(),
        );
    }

    let principales = activas.iter().filter(|p| p.principal).count();
    if principales > 1 {
        return Err("solo puede haber una pantalla principal".into());
    }
    if peticion.iter().any(|p| p.principal && !p.activa) {
        return Err("la pantalla principal no puede estar apagada".into());
    }

    for p in &activas {
        let salida = actual
            .iter()
            .find(|s| s.id == p.id)
            .expect("el id ya se comprobó contra el censo");

        if !escala_valida(p.escala) {
            return Err(format!(
                "escala {} no válida para «{}»: se admite entre {ESCALA_MIN} y {ESCALA_MAX} en pasos de {PASO_ESCALA}",
                p.escala, p.id
            ));
        }
        if !transformacion_valida(&p.transformacion) {
            return Err(format!(
                "rotación «{}» desconocida para «{}»",
                p.transformacion, p.id
            ));
        }
        if !salida
            .modos
            .iter()
            .any(|m| m.ancho == p.ancho && m.alto == p.alto && m.refresco_mhz == p.refresco_mhz)
        {
            return Err(format!(
                "«{}» no tiene el modo {}x{}@{}",
                p.id, p.ancho, p.alto, p.refresco_mhz
            ));
        }
        if p.vrr && !salida.vrr_capaz {
            return Err(format!("«{}» no admite frecuencia variable", p.id));
        }
        if p.x.abs() > POSICION_MAX || p.y.abs() > POSICION_MAX {
            return Err(format!("«{}» se coloca fuera del escritorio", p.id));
        }
    }

    // Solaparse a medias deja ventanas partidas entre dos pantallas y una zona
    // muerta donde el ratón desaparece. Clonar —misma posición y mismo tamaño
    // lógico— sí vale: es lo que se pide para proyectar.
    for (i, a) in activas.iter().enumerate() {
        let (aw, ah) = tamano_logico(a.ancho, a.alto, a.escala, &a.transformacion);
        for b in activas.iter().skip(i + 1) {
            let (bw, bh) = tamano_logico(b.ancho, b.alto, b.escala, &b.transformacion);
            let clonadas = a.x == b.x && a.y == b.y && aw == bw && ah == bh;
            let solapan = a.x < b.x + bw && b.x < a.x + aw && a.y < b.y + bh && b.y < a.y + ah;
            if solapan && !clonadas {
                return Err(format!(
                    "«{}» y «{}» se solapan a medias; ponlas juntas o clónalas del todo",
                    a.id, b.id
                ));
            }
        }
    }

    // Todas las pantallas extendidas deben formar una sola isla alcanzable.
    // Un hueco entre dos rectángulos crea una salida a la que el puntero no
    // puede llegar de forma continua. Tocar solo una esquina tampoco sirve:
    // el paso tendría un único punto lógico y sería prácticamente imposible
    // cruzarlo. Las salidas clonadas sí pertenecen al mismo grupo.
    if activas.len() > 1 {
        let mut alcanzables = vec![false; activas.len()];
        alcanzables[0] = true;
        loop {
            let mut cambio = false;
            for i in 0..activas.len() {
                if !alcanzables[i] {
                    continue;
                }
                for j in 0..activas.len() {
                    if !alcanzables[j] && salidas_conectadas(activas[i], activas[j]) {
                        alcanzables[j] = true;
                        cambio = true;
                    }
                }
            }
            if !cambio {
                break;
            }
        }
        if let Some((i, _)) = alcanzables.iter().enumerate().find(|(_, ok)| !**ok) {
            return Err(format!(
                "«{}» queda separada del resto del escritorio; coloca sus bordes en contacto",
                activas[i].id
            ));
        }
    }

    Ok(())
}

// 0.1.0
fn salidas_conectadas(a: &Peticion, b: &Peticion) -> bool {
    let (aw, ah) = tamano_logico(a.ancho, a.alto, a.escala, &a.transformacion);
    let (bw, bh) = tamano_logico(b.ancho, b.alto, b.escala, &b.transformacion);
    let clonadas = a.x == b.x && a.y == b.y && aw == bw && ah == bh;
    let borde_vertical = (a.x + aw == b.x || b.x + bw == a.x) && a.y < b.y + bh && b.y < a.y + ah;
    let borde_horizontal = (a.y + ah == b.y || b.y + bh == a.y) && a.x < b.x + bw && b.x < a.x + aw;
    clonadas || borde_vertical || borde_horizontal
}

/// Rellena lo que la petición no decide: si nadie se declaró principal, lo es
/// la primera encendida. Se hace después de validar para que el hueco no se
/// confunda con un error.
// 0.1.0
pub fn normalizar(peticion: &mut [Peticion]) {
    if !peticion.iter().any(|p| p.principal && p.activa)
        && let Some(p) = peticion.iter_mut().find(|p| p.activa)
    {
        p.principal = true;
    }

    // El origen del escritorio siempre es la principal. Así las superficies
    // propias del shell —panel, dock, bloqueo y OSD— conservan coordenadas
    // locales desde (0,0), mientras que una pantalla a la izquierda o arriba
    // usa coordenadas negativas de forma natural.
    if let Some(principal) = peticion.iter().find(|p| p.principal && p.activa) {
        let (ox, oy) = (principal.x, principal.y);
        for p in peticion.iter_mut().filter(|p| p.activa) {
            p.x -= ox;
            p.y -= oy;
        }
    }
}

// ── Persistencia ─────────────────────────────────────────────────────────

// 0.1.0
fn ruta() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("bookos").join("pantallas.conf"))
}

/// Fichero propio y no una clave de `panel.conf` porque esto **lo escribe el
/// escritorio**, no el usuario: se reescribe entero cada vez que se toca una
/// pantalla, y mezclarlo con la configuración escrita a mano acabaría pisando
/// los comentarios de alguien. Es la misma decisión que ya tomó
/// `launchpad.conf`.
// 0.1.0
pub fn serializar(peticion: &[Peticion]) -> String {
    let mut s = String::from(
        "# Pantallas de BookOS. Lo escribe el escritorio: se reescribe entero.\n\
         # Una línea por monitor, identificado por su EDID para que sobreviva a\n\
         # cambiar de puerto.\n",
    );
    for p in peticion {
        s.push_str(&format!(
            "salida = {}; activa={}; modo={}x{}@{}; escala={}; pos={},{}; rot={}; vrr={}; principal={}\n",
            p.id,
            si_no(p.activa),
            p.ancho,
            p.alto,
            p.refresco_mhz,
            p.escala,
            p.x,
            p.y,
            p.transformacion,
            si_no(p.vrr),
            si_no(p.principal),
        ));
    }
    s
}

// 0.1.0
fn si_no(v: bool) -> &'static str {
    if v { "si" } else { "no" }
}

/// Lo guardado, o vacío si no hay nada legible.
///
/// Una línea con una errata se descarta sola; el fichero entero se descarta si
/// lo que queda no encendería ninguna pantalla, que es la salvaguarda de la que
/// habla la cabecera del módulo.
// 0.1.0
pub fn deserializar(texto: &str) -> Vec<Peticion> {
    let mut salidas = Vec::new();
    for (n, linea) in texto.lines().enumerate() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        match linea_a_peticion(linea) {
            Some(p) => salidas.push(p),
            None => tracing::warn!(linea = n + 1, "línea de pantallas ilegible, se ignora"),
        }
    }
    if !salidas.iter().any(|p| p.activa) {
        if !salidas.is_empty() {
            tracing::warn!(
                "la configuración guardada no deja ninguna pantalla encendida: se ignora entera"
            );
        }
        return Vec::new();
    }
    salidas
}

// 0.1.0
fn linea_a_peticion(linea: &str) -> Option<Peticion> {
    let mut campos = linea.split(';').map(str::trim);
    let id = campos
        .next()?
        .strip_prefix("salida")?
        .trim_start_matches([' ', '='])
        .trim();
    if id.is_empty() {
        return None;
    }
    let mut p = Peticion {
        id: id.to_string(),
        activa: true,
        ancho: 0,
        alto: 0,
        refresco_mhz: 0,
        escala: 1.0,
        x: 0,
        y: 0,
        transformacion: "normal".into(),
        vrr: false,
        principal: false,
    };
    for campo in campos {
        let (clave, valor) = campo.split_once('=')?;
        match clave.trim() {
            "activa" => p.activa = booleano(valor)?,
            "modo" => {
                let (tam, hz) = valor.trim().split_once('@')?;
                let (w, h) = tam.split_once('x')?;
                p.ancho = w.trim().parse().ok()?;
                p.alto = h.trim().parse().ok()?;
                p.refresco_mhz = hz.trim().parse().ok()?;
            }
            "escala" => p.escala = valor.trim().parse().ok()?,
            "pos" => {
                let (x, y) = valor.trim().split_once(',')?;
                p.x = x.trim().parse().ok()?;
                p.y = y.trim().parse().ok()?;
            }
            "rot" => {
                let t = valor.trim();
                if !transformacion_valida(t) {
                    return None;
                }
                p.transformacion = t.to_string();
            }
            "vrr" => p.vrr = booleano(valor)?,
            "principal" => p.principal = booleano(valor)?,
            otra => tracing::warn!(clave = otra, "clave de pantalla desconocida, se ignora"),
        }
    }
    // Una salida encendida sin modo no se puede aplicar y no vale de nada
    // arrastrarla: mejor que el compositor elija el preferido.
    if p.activa && (p.ancho == 0 || p.alto == 0 || p.refresco_mhz == 0) {
        return None;
    }
    if !escala_valida(p.escala) {
        return None;
    }
    Some(p)
}

// 0.1.0
fn booleano(v: &str) -> Option<bool> {
    match v.trim() {
        "si" | "sí" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

// 0.1.0
pub fn cargar() -> Vec<Peticion> {
    let Some(ruta) = ruta() else {
        return Vec::new();
    };
    match std::fs::read_to_string(&ruta) {
        Ok(texto) => deserializar(&texto),
        Err(_) => Vec::new(),
    }
}

/// Guarda **atómicamente**: fichero temporal en el mismo directorio, `fsync` y
/// `rename`. Un corte de corriente a mitad de un `write` deja un
/// `pantallas.conf` truncado, y un truncado es exactamente el fichero que
/// enciende cero pantallas.
// 0.1.0
pub fn guardar(peticion: &[Peticion]) -> std::io::Result<()> {
    use std::io::Write;
    let Some(ruta) = ruta() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = ruta.with_extension("conf.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serializar(peticion).as_bytes())?;
        // El rename es atómico, pero solo garantiza que se ve el fichero
        // entero o el viejo: sin este sync el contenido puede llegar al disco
        // después del propio rename.
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &ruta)
}

// ── Aplicación, desde el hilo del compositor ─────────────────────────────

/// Convierte el censo actual en la petición equivalente. Es el punto de
/// partida al que se vuelve cuando una configuración nueva no cuela.
// 0.1.0
pub fn peticion_de(salidas: &[Salida]) -> Vec<Peticion> {
    salidas
        .iter()
        .map(|s| {
            let actual = s.modos.iter().find(|m| m.actual);
            Peticion {
                id: s.id.clone(),
                activa: s.activa,
                ancho: actual.map_or(0, |m| m.ancho),
                alto: actual.map_or(0, |m| m.alto),
                refresco_mhz: actual.map_or(0, |m| m.refresco_mhz),
                escala: s.escala,
                x: s.x,
                y: s.y,
                transformacion: s.transformacion.clone(),
                vrr: s.vrr,
                principal: s.principal,
            }
        })
        .collect()
}

/// Aplica una configuración: valida, la manda al hardware, la persiste y
/// reajusta lo que ve el usuario. Si el hardware la rechaza, se vuelve a la
/// anterior y **no** se guarda nada.
// 0.1.0
pub fn aplicar(
    state: &mut crate::state::BookosComp,
    mut peticion: Vec<Peticion>,
) -> Result<(), String> {
    let censo = state.pantallas.compartido.salidas();
    validar(&censo, &peticion)?;
    normalizar(&mut peticion);

    // El aplicador se saca del estado para poder llamarlo con `&mut state` en
    // la mano: es un cierre que vive dentro de la propia estructura que hay que
    // modificar después. Se devuelve siempre, incluso si el modeset falla.
    let Some(aplicador) = state.aplicar_pantallas.take() else {
        return Err("este backend no sabe reconfigurar las pantallas".into());
    };
    let resultado = aplicador(&peticion);
    let anterior = std::mem::take(&mut state.pantallas.ultima);

    let aplicado = match resultado {
        Ok(a) => a,
        Err(err) => {
            // Volver atrás con lo que sí funcionaba. Si tampoco se puede, no
            // hay nada más que hacer aquí: se avisa y se deja el hardware como
            // esté, que es lo que el usuario está viendo.
            if !anterior.is_empty()
                && let Err(err2) = aplicador(&anterior)
            {
                tracing::error!("no se pudo volver a la configuración anterior: {err2}");
            }
            state.pantallas.ultima = anterior;
            state.aplicar_pantallas = Some(aplicador);
            return Err(err);
        }
    };
    state.aplicar_pantallas = Some(aplicador);
    state.pantallas.ultima = peticion.clone();

    tras_aplicar(state, aplicado);
    // Una pantalla recién encendida nace con la curva identidad.
    crate::luz_nocturna::reponer(state);

    if let Err(err) = guardar(&peticion) {
        // Se aplicó y se ve: que no se pueda escribir el fichero es un fallo
        // real, pero no invalida lo que el usuario ya tiene delante.
        tracing::warn!("no se pudo guardar la configuración de pantallas: {err}");
    }
    Ok(())
}

/// Construye y aplica los cinco perfiles del selector Fn+F4. La decisión se
/// toma con el censo actual para no guardar resoluciones inventadas.
// 0.1.0
pub fn aplicar_modo_rapido(
    state: &mut crate::state::BookosComp,
    modo: bookos_shell::ModoProyeccion,
) -> Result<(), String> {
    use bookos_shell::ModoProyeccion::*;
    if modo == SinCambios {
        return Ok(());
    }
    let salidas = state.pantallas.compartido.salidas();
    if salidas.len() < 2 {
        return Err("No hay otra pantalla conectada".into());
    }
    let principal = salidas.iter().position(|s| s.principal).unwrap_or(0);
    let externa = (0..salidas.len()).find(|&i| i != principal).unwrap();
    let mut peticiones = peticion_de(&salidas);
    let poner_modo = |p: &mut Peticion, s: &Salida| -> Result<(), String> {
        let m = s
            .modos
            .iter()
            .find(|m| m.actual)
            .or_else(|| s.modos.iter().find(|m| m.preferido))
            .or_else(|| s.modos.first())
            .ok_or_else(|| format!("«{}» no anuncia ningún modo", s.id))?;
        p.ancho = m.ancho;
        p.alto = m.alto;
        p.refresco_mhz = m.refresco_mhz;
        Ok(())
    };
    for (i, p) in peticiones.iter_mut().enumerate() {
        p.activa = false;
        p.principal = false;
        if p.ancho == 0 {
            poner_modo(p, &salidas[i])?;
        }
    }
    match modo {
        Principal => {
            peticiones[principal].activa = true;
            peticiones[principal].principal = true;
        }
        Externa => {
            peticiones[externa].activa = true;
            peticiones[externa].principal = true;
            peticiones[externa].x = 0;
            peticiones[externa].y = 0;
        }
        Extender => {
            peticiones[principal].activa = true;
            peticiones[principal].principal = true;
            peticiones[principal].x = 0;
            peticiones[principal].y = 0;
            peticiones[externa].activa = true;
            let (w, _) = tamano_logico(
                peticiones[principal].ancho,
                peticiones[principal].alto,
                peticiones[principal].escala,
                &peticiones[principal].transformacion,
            );
            peticiones[externa].x = w;
            peticiones[externa].y = 0;
        }
        Duplicar => {
            let comun = salidas[principal]
                .modos
                .iter()
                .find(|a| {
                    salidas[externa]
                        .modos
                        .iter()
                        .any(|b| a.ancho == b.ancho && a.alto == b.alto)
                })
                .ok_or_else(|| {
                    "Las pantallas no comparten una resolución para duplicar".to_string()
                })?;
            for i in [principal, externa] {
                peticiones[i].activa = true;
                peticiones[i].ancho = comun.ancho;
                peticiones[i].alto = comun.alto;
                let m = salidas[i]
                    .modos
                    .iter()
                    .find(|m| m.ancho == comun.ancho && m.alto == comun.alto)
                    .unwrap();
                peticiones[i].refresco_mhz = m.refresco_mhz;
                peticiones[i].escala = 1.0;
                peticiones[i].x = 0;
                peticiones[i].y = 0;
            }
            peticiones[principal].principal = true;
        }
        SinCambios => unreachable!(),
    }
    aplicar(state, peticiones)
}

/// Reajusta el escritorio a un censo nuevo: dónde va cada salida, de qué tamaño
/// se dibuja el shell y qué escala se les anuncia a los clientes.
// 0.1.0
pub fn tras_aplicar(state: &mut crate::state::BookosComp, aplicado: Aplicado) {
    let Aplicado {
        salidas,
        mapa,
        principal,
    } = aplicado;

    // Las salidas que ya no están se desmapean antes de colocar las nuevas: si
    // no, una pantalla apagada seguiría ocupando su hueco en el `Space` y las
    // ventanas no volverían a la que queda encendida.
    let vivas: Vec<_> = mapa.iter().map(|(o, _)| o.clone()).collect();
    for output in state.space.outputs().cloned().collect::<Vec<_>>() {
        if !vivas.contains(&output) {
            state.space.unmap_output(&output);
        }
    }
    for (output, pos) in &mapa {
        state.space.map_output(output, *pos);
    }
    // Las ventanas guardadas de una pantalla que acaba de irse no pueden
    // quedarse esperándola: pasan a la que queda. Va aquí, con el `Space` ya
    // recolocado, porque el reencuadre necesita el área de destino de ahora.
    crate::escritorios::adoptar_huerfanas(state);

    if let Some((output, _)) = mapa.get(principal) {
        let escala = output.current_scale().fractional_scale();
        let modo = output.current_mode().map(|m| m.size).unwrap_or_default();
        // El shell razona en píxeles **físicos** del panel y su propia escala:
        // pasarle los lógicos lo dibujaría al tamaño correcto y con el texto
        // borroso, que es el fallo clásico de la escala fraccional.
        if let Some(shell) = state.shell.as_mut() {
            shell.resize(modo.w.max(1) as u32, modo.h.max(1) as u32, escala as f32);
            shell.refresh();
        }
        state.escala_forzada = Some(escala);
        state.broadcast_preferred_scale(escala);
        let escala_cursor = mapa
            .iter()
            .map(|(o, _)| o.current_scale().fractional_scale())
            .max_by(f64::total_cmp)
            .unwrap_or(escala);
        state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(
            escala_cursor,
            state.cursor_nominal,
        ));
    }

    state.pantallas.compartido.publicar(salidas);
    state.needs_redraw = true;
    crate::xwayland::actualizar_escala(state);
    crate::ajustes::avisar_salidas(state);
}

#[cfg(test)]
mod pruebas {

    #[test]
    fn el_monitor_se_llama_por_su_modelo_y_no_por_el_conector() {
        assert_eq!(nombre_visible("eDP-1", "ATNA40YK"), "Pantalla integrada");
        assert_eq!(nombre_visible("HDMI-A-1", "DELL U2720Q"), "DELL U2720Q");
        assert_eq!(nombre_visible("DP-2", "KMS"), "DP-2");
        assert_eq!(nombre_visible("DP-3", ""), "DP-3");
    }

    use super::*;

    fn modo(ancho: u32, alto: u32, hz: u32, actual: bool) -> Modo {
        Modo {
            ancho,
            alto,
            refresco_mhz: hz * 1000,
            preferido: hz == 120,
            actual,
        }
    }

    fn salida(id: &str) -> Salida {
        Salida {
            id: id.into(),
            conector: "eDP-1".into(),
            fabricante: "SDC".into(),
            modelo: "ATNA40YK".into(),
            serie: String::new(),
            mm_ancho: 300,
            mm_alto: 190,
            activa: true,
            modos: vec![
                modo(2880, 1800, 120, true),
                modo(2880, 1800, 90, false),
                modo(2880, 1800, 60, false),
                modo(1920, 1200, 60, false),
            ],
            escala: 1.75,
            escalas: ESCALAS.to_vec(),
            x: 0,
            y: 0,
            transformacion: "normal".into(),
            vrr_capaz: true,
            vrr: false,
            principal: true,
            logico_ancho: 1646,
            logico_alto: 1029,
        }
    }

    fn peticion(id: &str) -> Peticion {
        Peticion {
            id: id.into(),
            activa: true,
            ancho: 2880,
            alto: 1800,
            refresco_mhz: 120_000,
            escala: 1.75,
            x: 0,
            y: 0,
            transformacion: "normal".into(),
            vrr: false,
            principal: true,
        }
    }

    #[test]
    fn la_configuracion_de_serie_es_valida() {
        assert!(validar(&[salida("A")], &[peticion("A")]).is_ok());
    }

    #[test]
    fn nunca_se_pueden_apagar_todas_las_salidas() {
        let mut p = peticion("A");
        p.activa = false;
        p.principal = false;
        let err = validar(&[salida("A")], &[p]).unwrap_err();
        assert!(err.contains("ciegas"), "{err}");

        // Y tampoco con dos: apagar las dos es el mismo desastre.
        let (mut a, mut b) = (peticion("A"), peticion("B"));
        a.activa = false;
        a.principal = false;
        b.activa = false;
        b.principal = false;
        assert!(validar(&[salida("A"), salida("B")], &[a, b]).is_err());
    }

    #[test]
    fn la_principal_no_puede_estar_apagada() {
        let mut a = peticion("A");
        a.activa = false;
        let mut b = peticion("B");
        b.principal = false;
        b.x = 1646;
        assert!(validar(&[salida("A"), salida("B")], &[a, b]).is_err());
    }

    #[test]
    fn solo_hay_una_principal() {
        let a = peticion("A");
        let mut b = peticion("B");
        b.x = 1646;
        assert!(validar(&[salida("A"), salida("B")], &[a, b]).is_err());
    }

    #[test]
    fn las_escalas_fraccionales_de_la_lista_valen() {
        for e in ESCALAS {
            let mut p = peticion("A");
            p.escala = e;
            assert!(validar(&[salida("A")], &[p]).is_ok(), "escala {e}");
        }
        // Y las intermedias de 0,05, que es lo que se puede escribir a mano.
        for e in [1.05, 1.1, 1.15, 1.2, 1.3, 2.05] {
            assert!(escala_valida(e), "escala {e}");
        }
    }

    #[test]
    fn las_escalas_imposibles_no() {
        for e in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.1,
            8.0,
            1.7333,
        ] {
            assert!(!escala_valida(e), "escala {e} debería rechazarse");
            let mut p = peticion("A");
            p.escala = e;
            assert!(validar(&[salida("A")], &[p]).is_err(), "escala {e}");
        }
    }

    #[test]
    fn un_modo_que_no_existe_se_rechaza() {
        // Resolución inventada.
        let mut p = peticion("A");
        p.ancho = 3840;
        p.alto = 2160;
        assert!(validar(&[salida("A")], &[p]).is_err());

        // Resolución real pero a un refresco que este panel no da.
        let mut p = peticion("A");
        p.refresco_mhz = 144_000;
        assert!(validar(&[salida("A")], &[p]).is_err());

        // 90 y 120 Hz sí existen y tienen que pasar.
        for hz in [60_000, 90_000, 120_000] {
            let mut p = peticion("A");
            p.refresco_mhz = hz;
            assert!(validar(&[salida("A")], &[p]).is_ok(), "{hz} mHz");
        }
    }

    #[test]
    fn una_salida_desconocida_se_rechaza() {
        assert!(validar(&[salida("A")], &[peticion("Z")]).is_err());
    }

    #[test]
    fn no_se_admite_la_misma_salida_dos_veces() {
        let mut b = peticion("A");
        b.principal = false;
        assert!(validar(&[salida("A")], &[peticion("A"), b]).is_err());
    }

    #[test]
    fn el_vrr_solo_si_el_panel_puede() {
        let mut s = salida("A");
        s.vrr_capaz = false;
        let mut p = peticion("A");
        p.vrr = true;
        assert!(validar(&[s], &[p]).is_err());
        assert!(
            validar(
                &[salida("A")],
                &[{
                    let mut p = peticion("A");
                    p.vrr = true;
                    p
                }]
            )
            .is_ok()
        );
    }

    #[test]
    fn dos_pantallas_no_pueden_solaparse_a_medias() {
        let mut a = peticion("A");
        a.escala = 1.0;
        let mut b = peticion("B");
        b.principal = false;
        b.escala = 1.0;
        // 2880 de ancho a escala 1: pegada al borde vale, 100 px dentro no.
        b.x = 2880;
        assert!(validar(&[salida("A"), salida("B")], &[a.clone(), b.clone()]).is_ok());
        b.x = 2780;
        assert!(validar(&[salida("A"), salida("B")], &[a.clone(), b.clone()]).is_err());
        // Clonadas del todo sí: es lo que se pide para proyectar.
        b.x = 0;
        assert!(validar(&[salida("A"), salida("B")], &[a, b]).is_ok());
    }

    #[test]
    fn las_pantallas_pueden_ir_en_los_cuatro_lados() {
        let a = peticion("A");
        for (x, y) in [(-1646, 0), (1646, 0), (0, -1029), (0, 1029)] {
            let mut b = peticion("B");
            b.principal = false;
            b.x = x;
            b.y = y;
            assert!(
                validar(&[salida("A"), salida("B")], &[a.clone(), b]).is_ok(),
                "posición {x},{y}"
            );
        }
    }

    #[test]
    fn una_isla_separada_o_unida_solo_por_la_esquina_se_rechaza() {
        let a = peticion("A");
        let mut b = peticion("B");
        b.principal = false;
        b.x = 2000;
        assert!(validar(&[salida("A"), salida("B")], &[a.clone(), b.clone()]).is_err());

        b.x = 1646;
        b.y = 1029;
        assert!(validar(&[salida("A"), salida("B")], &[a, b]).is_err());
    }

    #[test]
    fn la_rotacion_desconocida_se_rechaza() {
        let mut p = peticion("A");
        p.transformacion = "45".into();
        assert!(validar(&[salida("A")], &[p]).is_err());
        for t in [
            "normal",
            "90",
            "180",
            "270",
            "flipped",
            "flipped-90",
            "flipped-180",
            "flipped-270",
        ] {
            let mut p = peticion("A");
            p.transformacion = t.into();
            assert!(validar(&[salida("A")], &[p]).is_ok(), "{t}");
        }
    }

    #[test]
    fn girar_un_cuarto_intercambia_el_tamano_logico() {
        assert_eq!(tamano_logico(2880, 1800, 1.0, "normal"), (2880, 1800));
        assert_eq!(tamano_logico(2880, 1800, 1.0, "90"), (1800, 2880));
        assert_eq!(tamano_logico(2880, 1800, 1.0, "180"), (2880, 1800));
        // 2880/1,75 = 1645,71: hacia arriba, para no dejar una fila sin pintar.
        assert_eq!(tamano_logico(2880, 1800, 1.75, "normal"), (1646, 1029));
    }

    #[test]
    fn la_ida_y_vuelta_del_fichero_conserva_todo() {
        let p = vec![peticion("SDC-ATNA40YK-0"), {
            let mut b = peticion("DEL-U2720Q-ABC123");
            b.principal = false;
            b.activa = false;
            b.x = 1646;
            b.transformacion = "270".into();
            b
        }];
        let leido = deserializar(&serializar(&p));
        assert_eq!(leido, p);
    }

    #[test]
    fn un_fichero_que_apaga_todo_se_ignora_entero() {
        let texto = "salida = A; activa=no; modo=2880x1800@120000; escala=1.75; pos=0,0; rot=normal; vrr=no; principal=no\n";
        assert!(deserializar(texto).is_empty());
    }

    #[test]
    fn una_linea_rota_no_se_lleva_por_delante_a_las_demas() {
        let texto = "salida = A; activa=si; modo=2880x1800@120000; escala=1.75; pos=0,0; rot=normal; vrr=no; principal=si\n\
                     esto no es una linea de salida\n\
                     salida = B; activa=si; modo=chorizo; escala=1.0; pos=0,0; rot=normal; vrr=no; principal=no\n";
        let leido = deserializar(texto);
        assert_eq!(leido.len(), 1);
        assert_eq!(leido[0].id, "A");
    }

    #[test]
    fn una_escala_absurda_en_el_fichero_descarta_esa_linea() {
        let texto = "salida = A; activa=si; modo=2880x1800@120000; escala=0; pos=0,0; rot=normal; vrr=no; principal=si\n";
        assert!(deserializar(texto).is_empty());
    }

    #[test]
    fn sin_principal_se_elige_la_primera_encendida() {
        let mut p = vec![
            {
                let mut a = peticion("A");
                a.activa = false;
                a.principal = false;
                a
            },
            {
                let mut b = peticion("B");
                b.principal = false;
                b
            },
        ];
        normalizar(&mut p);
        assert!(!p[0].principal);
        assert!(p[1].principal);
    }

    #[test]
    fn la_principal_define_el_origen_del_escritorio() {
        let mut p = vec![
            {
                let mut a = peticion("A");
                a.x = 1646;
                a
            },
            {
                let mut b = peticion("B");
                b.principal = false;
                b.x = 0;
                b
            },
        ];
        normalizar(&mut p);
        assert_eq!((p[0].x, p[0].y), (0, 0));
        assert_eq!((p[1].x, p[1].y), (-1646, 0));
    }
}
