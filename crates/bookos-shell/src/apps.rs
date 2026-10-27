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

pub struct App {
    pub nombre: String,
    /// Ya limpio de los códigos `%f`, `%u`… de la especificación.
    pub exec: String,
    pub icono: String,
    /// El nombre en minúsculas y sin acentos, para buscar sin repetir el
    /// trabajo en cada pulsación de tecla.
    pub(crate) normalizado: String,
}

/// Dónde busca la especificación de freedesktop, de más prioritario a menos.
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
pub fn leer() -> Vec<App> {
    let mut vistos = HashSet::new();
    let mut apps = Vec::new();

    for dir in directorios() {
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
    apps
}

/// La aplicación que corresponde a un `app_id` de Wayland.
///
/// Se prueban las rutas que el `app_id` implica —`org.kde.konsole.desktop`, y
/// el último componente, `konsole.desktop`— antes de recorrer todo el
/// directorio: acertar es un `stat`, y solo se paga la lectura completa cuando
/// el `.desktop` no se llama como la aplicación dice llamarse.
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
pub(crate) fn leer_lanzador(ruta: &std::path::Path) -> Option<App> {
    leer_entrada(ruta).map(|(app, _)| app)
}

/// Un `Icon=` con ruta absoluta se salta el tema de iconos y sale sin teñir
/// con los pack Tinted (Zen trae `.../default128.png`). Si el tema tiene un
/// icono con el nombre de su `StartupWMClass`, gana ese.
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
fn leer_entrada(ruta: &std::path::Path) -> Option<(App, bool)> {
    let texto = std::fs::read_to_string(ruta).ok()?;

    let mut nombre = None;
    let mut nombre_es = None;
    let mut exec = None;
    let mut icono = None;
    let mut oculta = false;
    let mut sin_menu = false;
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
            "Icon" => icono = Some(valor.trim().to_string()),
            // `Hidden` es «esta entrada está borrada»; `NoDisplay`, solo «no
            // en los menús».
            "Hidden" => oculta = valor.trim() == "true",
            "NoDisplay" => sin_menu = valor.trim() == "true",
            "Type" => es_app = valor.trim() == "Application",
            _ => {}
        }
    }

    if oculta || !es_app {
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

/// Quita los códigos de campo de la especificación.
///
/// `Exec=firefox %u` lanzado tal cual le pasa un `%u` literal a Firefox. Se
/// quitan todos los `%x` menos `%%`, que es un porcentaje de verdad.
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

    #[test]
    fn wm_class_se_lee_solo_de_la_entrada_principal() {
        let texto = "[Desktop Entry]\nName=VirtualBox\nStartupWMClass=VirtualBox Manager\n\
                     [Desktop Action Otra]\nStartupWMClass=Incorrecta\n";
        assert_eq!(startup_wm_class(texto), Some("VirtualBox Manager"));
    }
}
