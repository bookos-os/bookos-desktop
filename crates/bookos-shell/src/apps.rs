//! La lista de aplicaciones instaladas, leída de los `.desktop`.
//!
//! El launchpad de Plasma usa `Kicker.RootModel`, que fuera de plasmashell no
//! existe. Aquí se leen los ficheros directamente, que es lo que hace Kicker
//! por debajo: son INI y la especificación de entradas de escritorio de
//! freedesktop es corta.
//!
//! Se lee **una vez**, al abrir el launchpad. Vigilar los directorios con
//! inotify para enterarse de una instalación sería un descriptor abierto y un
//! despertar por cada `dnf install` a cambio de nada: el launchpad se cierra y
//! se vuelve a abrir en un segundo.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Clone)]
pub struct App {
    pub nombre: String,
    /// Ya limpio de los códigos `%f`, `%u`… de la especificación.
    pub exec: String,
    pub icono: String,
    /// El nombre en minúsculas y sin acentos, para buscar sin repetir el
    /// trabajo en cada pulsación de tecla.
    pub(crate) normalizado: String,
}

type CacheApps = (Vec<(PathBuf, Option<SystemTime>)>, Vec<App>);
static CACHE_APPS: OnceLock<Mutex<Option<CacheApps>>> = OnceLock::new();

fn huella_directorios(dirs: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>)> {
    dirs.iter()
        .map(|d| {
            let mtime = std::fs::metadata(d).and_then(|m| m.modified()).ok();
            (d.clone(), mtime)
        })
        .collect()
}

/// Dónde busca la especificación de freedesktop, de más prioritario a menos.
// 0.1.0
fn directorios() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
    }
    dirs.push(PathBuf::from("/usr/local/share/applications"));
    dirs.push(PathBuf::from("/usr/share/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

/// Todas las aplicaciones visibles, ordenadas por nombre.
///
/// Cachea el catálogo por la fecha de modificación de los directorios de
/// aplicaciones (4 `stat()` en lugar de releer cientos de `.desktop` al abrir el
/// Buscador o Launchpad).
// 0.1.0
pub fn leer() -> Vec<App> {
    let dirs = directorios();
    let huella = huella_directorios(&dirs);
    let cache = CACHE_APPS.get_or_init(Default::default);
    if let Ok(guard) = cache.lock()
        && let Some((prev_huella, apps)) = guard.as_ref()
        && *prev_huella == huella
    {
        return apps.clone();
    }

    let mut vistos = HashSet::new();
    let mut apps = Vec::new();

    for dir in dirs {
        let Ok(entradas) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entrada in entradas.filter_map(|e| e.ok()) {
            let ruta = entrada.path();
            if ruta.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            // El nombre del fichero es el identificador: el mismo id en un
            // directorio de más prioridad gana, que es como el usuario
            // sustituye un lanzador del sistema por el suyo.
            let Some(id) = ruta.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if !vistos.insert(id) {
                continue;
            }
            if let Some(app) = leer_una(&ruta) {
                apps.push(app);
            }
        }
    }

    apps.sort_by(|a, b| a.normalizado.cmp(&b.normalizado));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((huella, apps.clone()));
    }
    apps
}

/// La aplicación que corresponde a un `app_id` de Wayland.
///
/// Se prueban las rutas que el `app_id` implica —`org.kde.konsole.desktop`, y
/// el último componente, `konsole.desktop`— antes de recorrer todo el
/// directorio: acertar es un `stat`, y solo se paga la lectura completa cuando
/// el `.desktop` no se llama como la aplicación dice llamarse.
// 0.1.0
pub fn por_app_id(app_id: &str) -> Option<App> {
    let cola = app_id.rsplit('.').next().unwrap_or(app_id);
    // En minúsculas también: VirtualBox en Wayland se anuncia como
    // `VirtualBox` y su fichero es `virtualbox.desktop`. Su `StartupWMClass`
    // —«VirtualBox Manager», abajo— es el nombre X11 y tampoco casa.
    let minusculas = cola.to_lowercase();
    let dirs = directorios();
    for dir in &dirs {
        for nombre in [app_id, cola, &minusculas] {
            let ruta = dir.join(format!("{nombre}.desktop"));
            if let Some(app) = leer_una(&ruta) {
                return Some(app);
            }
        }
    }

    // Las aplicaciones X11 se identifican por WM_CLASS, que no tiene por qué
    // parecerse al nombre del fichero. VirtualBox es el caso importante:
    // `virtualbox.desktop` declara `StartupWMClass=VirtualBox Manager`. Sin
    // mirar esa clave el dock conoce la ventana, pero no encuentra su icono y
    // cae a la baldosa con una inicial.
    for dir in dirs {
        let Ok(entradas) = std::fs::read_dir(dir) else {
            continue;
        };
        for ruta in entradas
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
        {
            let Ok(texto) = std::fs::read_to_string(&ruta) else {
                continue;
            };
            if startup_wm_class(&texto).is_some_and(|c| c.eq_ignore_ascii_case(app_id))
                && let Some(app) = leer_una(&ruta)
            {
                return Some(app);
            }
        }
    }
    None
}

/// `StartupWMClass` del grupo principal de una entrada de escritorio.
// 0.1.0
fn startup_wm_class(texto: &str) -> Option<&str> {
    let mut en_entrada = false;
    for linea in texto.lines().map(str::trim) {
        if linea.starts_with('[') {
            en_entrada = linea == "[Desktop Entry]";
            continue;
        }
        if en_entrada && let Some(valor) = linea.strip_prefix("StartupWMClass=") {
            return Some(valor.trim());
        }
    }
    None
}

// 0.1.0
pub(crate) fn leer_una(ruta: &std::path::Path) -> Option<App> {
    leer_entrada(ruta).and_then(|(app, sin_menu)| (!sin_menu).then_some(app))
}

/// Un lanzador puesto a mano en el escritorio, aunque diga `NoDisplay=true`.
///
/// `NoDisplay` solo significa «no lo enseñes en los menús», y hay lanzadores
/// que lo llevan justo para que únicamente se vean donde alguien los coloca:
/// el `liveinst.desktop` de Anaconda es uno. Descartarlo aquí hacía que el
/// escritorio cayera a `xdg-open`, y el lanzador se abría en el bloc de notas
/// como texto (`application/x-desktop` hereda de `text/plain`).
// 0.1.0
pub(crate) fn leer_lanzador(ruta: &std::path::Path) -> Option<App> {
    leer_entrada(ruta).map(|(app, _)| app)
}

/// Un `Icon=` con ruta absoluta se salta el tema de iconos y sale sin teñir
/// con los pack Tinted (Zen trae `.../default128.png`). Si el tema tiene un
/// icono con el nombre de su `StartupWMClass`, gana ese.
// 0.1.0
fn icono_del_tema(icono: Option<String>, clase: Option<&str>) -> Option<String> {
    let clase = clase.map(str::to_lowercase);
    match (icono, clase) {
        (Some(ruta), Some(clase)) if ruta.starts_with('/') => {
            Some(crate::icono::resolve_icon(&clase).map_or(ruta, |_| clase))
        }
        (icono, _) => icono,
    }
}

/// La aplicación y si pide quedarse fuera de los menús (`NoDisplay`).
// 0.1.0
// modificado en la 0.2.0 -- respeta TryExec
fn leer_entrada(ruta: &std::path::Path) -> Option<(App, bool)> {
    let texto = std::fs::read_to_string(ruta).ok()?;

    let mut nombre = None;
    let mut nombre_es = None;
    let mut exec = None;
    let mut icono = None;
    let mut oculta = false;
    let mut sin_menu = false;
    let mut prueba = None;
    let mut es_app = false;
    let mut en_entrada = false;

    for linea in texto.lines() {
        let linea = linea.trim();
        // Un `.desktop` puede traer varios grupos —las "acciones" del menú
        // contextual— y solo el principal describe la aplicación. Sin esto, un
        // `Name` de una acción pisaba el de la app.
        if linea.starts_with('[') {
            en_entrada = linea == "[Desktop Entry]";
            continue;
        }
        if !en_entrada {
            continue;
        }
        let Some((clave, valor)) = linea.split_once('=') else {
            continue;
        };
        match clave.trim() {
            // Las traducciones vienen como `Name[es]`; se ignoran a propósito
            // hasta que el shell tenga idioma propio, para no coger la primera
            // que aparezca en el fichero.
            "Name" => nombre = Some(valor.trim().to_string()),
            "Name[es]" => nombre_es = Some(valor.trim().to_string()),
            "Exec" => exec = Some(limpiar_exec(valor.trim())),
            "TryExec" => prueba = Some(valor.trim().to_string()),
            "Icon" => icono = Some(valor.trim().to_string()),
            // `Hidden` es «esta entrada está borrada»; `NoDisplay`, solo «no
            // en los menús».
            "Hidden" => oculta = valor.trim() == "true",
            "NoDisplay" => sin_menu = valor.trim() == "true",
            "Type" => es_app = valor.trim() == "Application",
            _ => {}
        }
    }

    // `TryExec` es «enséñame solo si este programa existe» (especificación
    // de freedesktop). Sin respetarlo, un lanzador sin su programa —el
    // `bookos-captura.desktop` que se instalaba sin `bookos-captura`— salía en
    // el buscador por delante de la acción de captura, y no hacía nada.
    if oculta || !es_app || prueba.is_some_and(|p| !existe_programa(&p)) {
        return None;
    }
    let nombre = if std::env::var("LANG").unwrap_or_default().starts_with("es") {
        nombre_es.or(nombre)
    } else {
        nombre
    }?;
    let exec = exec.filter(|e| !e.is_empty())?;
    let normalizado = normalizar(&nombre);
    Some((
        App {
            icono: icono_del_tema(icono, startup_wm_class(&texto))
                .unwrap_or_else(|| nombre.clone()),
            nombre,
            exec,
            normalizado,
        },
        sin_menu,
    ))
}

/// ¿Existe el programa? Una ruta absoluta se mira tal cual; un nombre suelto,
/// en cada carpeta del `PATH`.
// añadido en la 0.2.0
pub fn existe_programa(programa: &str) -> bool {
    if programa.is_empty() {
        return false;
    }
    if programa.starts_with('/') {
        return std::path::Path::new(programa).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(programa).is_file()))
}

/// Quita los códigos de campo de la especificación.
///
/// `Exec=firefox %u` lanzado tal cual le pasa un `%u` literal a Firefox. Se
/// quitan todos los `%x` menos `%%`, que es un porcentaje de verdad.
// 0.1.0
fn limpiar_exec(exec: &str) -> String {
    let mut salida = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            salida.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => salida.push('%'),
            // %f %F %u %U %i %c %k y cualquier otro: fuera.
            Some(_) => {}
            None => {}
        }
    }
    salida.trim().to_string()
}

/// Minúsculas y sin diacríticos, para que "cámara" case con "camara".
///
/// No es una normalización Unicode de verdad —eso sería NFD y una tabla— sino
/// la lista de vocales acentuadas y la eñe, que es lo que aparece en los
/// nombres de aplicaciones que vamos a ver.
// 0.1.0
// modified 0.1.1 -- pública para el buscador
pub fn normalizar(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            otro => otro,
        })
        .collect()
}

/// Cuánto encaja `consulta` con la app. Menor es mejor; `None` si no encaja.
///
/// La escala es la del launchpad de Plasma, y el orden importa más que los
/// números: lo que se busca es que escribir "fir" ponga Firefox el primero y no
/// una aplicación cualquiera que lleve "fir" a mitad de palabra.
// 0.1.0
pub fn puntuar(app: &App, consulta: &str) -> Option<u32> {
    let consulta = normalizar(consulta);
    if consulta.is_empty() {
        return Some(0);
    }
    let nombre = &app.normalizado;

    if *nombre == consulta {
        return Some(0);
    }
    if nombre.starts_with(&consulta) {
        return Some(1);
    }
    // Prefijo de cualquier palabra: "web" encuentra "Navegador Web".
    if nombre.split_whitespace().any(|p| p.starts_with(&consulta)) {
        return Some(2);
    }
    // Iniciales: "vsc" encuentra "Visual Studio Code".
    let iniciales: String = nombre
        .split_whitespace()
        .filter_map(|p| p.chars().next())
        .collect();
    if iniciales.starts_with(&consulta) {
        return Some(3);
    }
    if nombre.contains(&consulta) {
        return Some(4);
    }
    // Subsecuencia difusa: las letras en orden aunque no seguidas.
    let mut cs = consulta.chars();
    let mut actual = cs.next();
    for c in nombre.chars() {
        if Some(c) == actual {
            actual = cs.next();
        }
    }
    actual.is_none().then_some(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(nombre: &str) -> App {
        App {
            normalizado: normalizar(nombre),
            nombre: nombre.to_string(),
            exec: "x".into(),
            icono: "x".into(),
        }
    }

    #[test]
    fn los_codigos_de_campo_no_llegan_al_programa() {
        assert_eq!(limpiar_exec("firefox %u"), "firefox");
        assert_eq!(limpiar_exec("kate %F"), "kate");
        assert_eq!(
            limpiar_exec("env FOO=1 app -x %U --flag"),
            "env FOO=1 app -x  --flag"
        );
        // `%%` sí es un porcentaje de verdad.
        assert_eq!(limpiar_exec("cosa --al 50%%"), "cosa --al 50%");
    }

    #[test]
    fn leer_cachea_catalogo_entre_llamadas_consecutivas() {
        let primera = leer();
        let segunda = leer();
        let nombres1: Vec<&str> = primera.iter().map(|a| a.nombre.as_str()).collect();
        let nombres2: Vec<&str> = segunda.iter().map(|a| a.nombre.as_str()).collect();
        assert_eq!(nombres1, nombres2);
    }

    #[test]
    fn la_puntuacion_ordena_como_se_espera() {
        let firefox = app("Firefox");
        let navegador = app("Navegador Web");
        let vsc = app("Visual Studio Code");

        // Prefijo gana a substring.
        assert!(puntuar(&firefox, "fir") < puntuar(&navegador, "web").map(|_| 99));
        assert_eq!(puntuar(&firefox, "firefox"), Some(0));
        assert_eq!(puntuar(&firefox, "fire"), Some(1));
        assert_eq!(puntuar(&navegador, "web"), Some(2));
        assert_eq!(puntuar(&vsc, "vsc"), Some(3));
        assert_eq!(puntuar(&firefox, "efo"), Some(4));
        // Y lo que no está, no está.
        assert_eq!(puntuar(&firefox, "zzz"), None);
    }

    #[test]
    fn se_busca_sin_acentos() {
        let camara = app("Cámara");
        assert_eq!(puntuar(&camara, "camara"), Some(0));
        assert_eq!(puntuar(&camara, "cam"), Some(1));
    }

    /// El lanzador del instalador lleva `NoDisplay=true`: fuera del launchpad,
    /// pero en el escritorio tiene que ejecutarse y no abrirse como texto.
    #[test]
    fn no_display_sale_del_menu_pero_se_lanza_desde_el_escritorio() {
        let ruta = std::env::temp_dir().join(format!(
            "bookos-test-liveinst-{}.desktop",
            std::process::id()
        ));
        std::fs::write(
            &ruta,
            "[Desktop Entry]\nName=Install to Hard Drive\nExec=liveinst\nType=Application\nNoDisplay=true\n",
        )
        .expect("escribir el .desktop de prueba");
        let en_menu = leer_una(&ruta);
        let lanzador = leer_lanzador(&ruta).map(|app| app.exec);
        let _ = std::fs::remove_file(&ruta);
        assert!(en_menu.is_none());
        assert_eq!(lanzador.as_deref(), Some("liveinst"));
    }

    /// Un lanzador cuyo `TryExec` no existe no sale: es el caso de
    /// `bookos-captura`, que ganaba a la acción de captura en el buscador.
    #[test]
    fn try_exec_que_no_existe_quita_la_entrada() {
        let ruta = std::env::temp_dir().join(format!(
            "bookos-test-tryexec-{}.desktop",
            std::process::id()
        ));
        std::fs::write(
            &ruta,
            "[Desktop Entry]\nName=Captura\nExec=bookos-programa-que-no-existe\n\
             TryExec=bookos-programa-que-no-existe\nType=Application\n",
        )
        .expect("escribir el .desktop de prueba");
        let sin_programa = leer_una(&ruta);
        std::fs::write(
            &ruta,
            "[Desktop Entry]\nName=Shell\nExec=sh\nTryExec=sh\nType=Application\n",
        )
        .expect("escribir el .desktop de prueba");
        let con_programa = leer_una(&ruta);
        let _ = std::fs::remove_file(&ruta);
        assert!(sin_programa.is_none());
        assert!(con_programa.is_some());
    }

    #[test]
    fn wm_class_se_lee_solo_de_la_entrada_principal() {
        let texto = "[Desktop Entry]\nName=VirtualBox\nStartupWMClass=VirtualBox Manager\n\
                     [Desktop Action Otra]\nStartupWMClass=Incorrecta\n";
        assert_eq!(startup_wm_class(texto), Some("VirtualBox Manager"));
    }
}

/// El tipo MIME de un archivo por su extensión, con la tabla de
/// shared-mime-info (`globs2`). Sin abrir el fichero ni lanzar `xdg-mime`
/// (unos 20 ms por llamada, y el menú se arma al hacer clic): para elegir
/// con qué se abre basta con lo que dice el nombre.
// añadido en la 0.2.3
pub fn mime_de(ruta: &std::path::Path) -> Option<String> {
    static GLOBS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    if ruta.is_dir() {
        return Some("inode/directory".into());
    }
    let nombre = ruta.file_name()?.to_string_lossy().to_lowercase();
    let globs = GLOBS.get_or_init(|| {
        let mut tabla: Vec<(u32, String, String)> = ["/usr/share/mime/globs2", "/usr/local/share/mime/globs2"]
            .iter()
            .filter_map(|r| std::fs::read_to_string(r).ok())
            .flat_map(|texto| {
                texto
                    .lines()
                    .filter(|l| !l.starts_with('#'))
                    .filter_map(|l| {
                        let mut partes = l.splitn(3, ':');
                        let peso = partes.next()?.parse().ok()?;
                        let mime = partes.next()?.to_string();
                        // Solo los de sufijo: `*.pdf`. Los nombres exactos
                        // (`Makefile`) no aparecen en un escritorio.
                        let sufijo = partes.next()?.strip_prefix('*')?.to_lowercase();
                        (!sufijo.contains(['*', '?', '['])).then_some((peso, mime, sufijo))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        // El sufijo más largo gana (`.tar.gz` antes que `.gz`), y a igual
        // largo, el de más peso.
        tabla.sort_by(|a, b| b.2.len().cmp(&a.2.len()).then(b.0.cmp(&a.0)));
        tabla.into_iter().map(|(_, m, s)| (s, m)).collect()
    });
    globs
        .iter()
        .find(|(sufijo, _)| nombre.ends_with(sufijo.as_str()))
        .map(|(_, mime)| mime.clone())
}

/// Las aplicaciones que dicen abrir ese tipo (`MimeType=`), con la
/// predeterminada de `mimeapps.list` la primera. Como mucho `max`.
// añadido en la 0.2.3
pub fn para_tipo(mime: &str, max: usize) -> Vec<App> {
    let predeterminada = predeterminada(mime);
    let mut vistos = HashSet::new();
    let mut apps: Vec<(bool, App)> = Vec::new();
    for dir in directorios() {
        let Ok(entradas) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ruta in entradas.filter_map(|e| e.ok()).map(|e| e.path()) {
            let Some(id) = ruta.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if !id.ends_with(".desktop") || !vistos.insert(id.clone()) {
                continue;
            }
            let Ok(texto) = std::fs::read_to_string(&ruta) else {
                continue;
            };
            let abre = texto
                .lines()
                .find_map(|l| l.trim().strip_prefix("MimeType="))
                .is_some_and(|tipos| tipos.split(';').any(|t| t.trim() == mime));
            if abre && let Some(app) = leer_una(&ruta) {
                apps.push((predeterminada.as_deref() == Some(id.as_str()), app));
            }
        }
    }
    apps.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.normalizado.cmp(&b.1.normalizado)));
    apps.into_iter().take(max).map(|(_, app)| app).collect()
}

/// El `.desktop` predeterminado para un tipo, de `~/.config/mimeapps.list`
/// y luego del del sistema.
// añadido en la 0.2.3
fn predeterminada(mime: &str) -> Option<String> {
    let mut rutas = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        rutas.push(PathBuf::from(home).join(".config/mimeapps.list"));
    }
    rutas.push(PathBuf::from("/usr/share/applications/mimeapps.list"));
    rutas.iter().find_map(|ruta| {
        let texto = std::fs::read_to_string(ruta).ok()?;
        let mut en_grupo = false;
        texto.lines().map(str::trim).find_map(|linea| {
            if linea.starts_with('[') {
                en_grupo = linea == "[Default Applications]";
                return None;
            }
            let valor = linea.strip_prefix(mime)?.strip_prefix('=')?;
            en_grupo
                .then(|| valor.split(';').next().map(str::to_string))
                .flatten()
        })
    })
}

/// Las acciones propias de una app (`[Desktop Action …]` de su `.desktop`):
/// «Nueva ventana privada» del navegador. `(nombre, exec)`.
// añadido en la 0.2.3
pub fn acciones(app_id: &str) -> Vec<(String, String)> {
    let cola = app_id.rsplit('.').next().unwrap_or(app_id);
    let Some(texto) = directorios().iter().find_map(|dir| {
        [app_id, cola]
            .iter()
            .find_map(|n| std::fs::read_to_string(dir.join(format!("{n}.desktop"))).ok())
    }) else {
        return Vec::new();
    };
    let es = std::env::var("LANG").unwrap_or_default().starts_with("es");
    let mut acciones = Vec::new();
    let (mut en_accion, mut nombre, mut nombre_es, mut exec) = (false, None, None, None);
    let mut cerrar = |nombre: &mut Option<String>, nombre_es: &mut Option<String>, exec: &mut Option<String>| {
        let elegido = if es { nombre_es.take().or(nombre.take()) } else { nombre.take() };
        if let (Some(n), Some(e)) = (elegido, exec.take()) {
            acciones.push((n, e));
        }
        *nombre = None;
        *nombre_es = None;
    };
    for linea in texto.lines().map(str::trim) {
        if linea.starts_with('[') {
            if en_accion {
                cerrar(&mut nombre, &mut nombre_es, &mut exec);
            }
            en_accion = linea.starts_with("[Desktop Action ");
            continue;
        }
        if !en_accion {
            continue;
        }
        match linea.split_once('=') {
            Some(("Name", v)) => nombre = Some(v.trim().to_string()),
            Some(("Name[es]", v)) => nombre_es = Some(v.trim().to_string()),
            Some(("Exec", v)) => exec = Some(limpiar_exec(v.trim())),
            _ => {}
        }
    }
    if en_accion {
        cerrar(&mut nombre, &mut nombre_es, &mut exec);
    }
    acciones
}

/// Los archivos que esa app abrió hace poco, del registro común de
/// freedesktop (`recently-used.xbel`, lo escriben GTK y KDE). Lo más reciente
/// primero, solo lo que sigue existiendo, como mucho `max`.
///
/// Se casa por el **programa** del `exec` y no por el nombre: el registro
/// guarda `name=""` para Dolphin, pero siempre lleva la orden.
// añadido en la 0.2.3
pub fn recientes(exec: &str, max: usize) -> Vec<PathBuf> {
    let programa = |orden: &str| {
        orden
            .trim_matches('\'')
            .split_whitespace()
            .next()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
    };
    let Some(suyo) = programa(exec) else {
        return Vec::new();
    };
    let Some(ruta) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("recently-used.xbel"))
    else {
        return Vec::new();
    };
    let Ok(texto) = std::fs::read_to_string(ruta) else {
        return Vec::new();
    };
    let mut hallados: Vec<(String, PathBuf)> = texto
        .split("<bookmark ")
        .skip(1)
        .filter_map(|bloque| {
            let atributo = |nombre: &str, en: &str| {
                let resto = en.split_once(&format!("{nombre}=\""))?.1;
                Some(resto.split_once('"')?.0.to_string())
            };
            let href = atributo("href", bloque)?;
            let fecha = atributo("modified", bloque).unwrap_or_default();
            let es_suyo = bloque.split("<bookmark:application ").skip(1).any(|app| {
                atributo("exec", app).and_then(|e| programa(&e)).as_deref() == Some(suyo.as_str())
            });
            let ruta = PathBuf::from(desescapar_uri(href.strip_prefix("file://")?));
            (es_suyo && ruta.exists()).then_some((fecha, ruta))
        })
        .collect();
    // La fecha va en ISO 8601: ordenar el texto es ordenar el tiempo.
    hallados.sort_by(|a, b| b.0.cmp(&a.0));
    hallados.into_iter().map(|(_, r)| r).take(max).collect()
}

/// `%20` y compañía de vuelta a bytes. Lo que no sea un escape válido se deja.
// añadido en la 0.2.3
fn desescapar_uri(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut salida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                salida.push(b);
                i += 3;
            }
            (b, _) => {
                salida.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&salida).into_owned()
}

/// La ruta del lanzador de inicio de sesión de una app.
// añadido en la 0.2.3
fn ruta_autostart(app_id: &str) -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart").join(format!("{app_id}.desktop")))
}

/// ¿Se abre al iniciar sesión? Lo que manda es `~/.config/autostart`, que es
/// lo que leen todos los escritorios (y también la sesión de BookUI).
// añadido en la 0.2.3
pub fn abre_al_iniciar(app_id: &str) -> bool {
    ruta_autostart(app_id).is_some_and(|r| r.exists())
}

/// Pone o quita la app del inicio de sesión. Se copia su `.desktop` tal cual:
/// un autostart es un lanzador normal en otra carpeta.
// añadido en la 0.2.3
pub fn poner_al_iniciar(app_id: &str, abrir: bool) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let destino = ruta_autostart(app_id)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "sin HOME ni XDG_CONFIG_HOME"))?;
    if !abrir {
        return match std::fs::remove_file(&destino) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            otro => otro,
        };
    }
    let cola = app_id.rsplit('.').next().unwrap_or(app_id);
    let origen = directorios()
        .iter()
        .flat_map(|d| [d.join(format!("{app_id}.desktop")), d.join(format!("{cola}.desktop"))])
        .find(|r| r.exists())
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "la app no tiene .desktop"))?;
    if let Some(dir) = destino.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(origen, destino).map(|_| ())
}
