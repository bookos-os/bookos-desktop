//! Lectura del estado del equipo y limpieza de lo que pertenece al usuario.
//!
//! Va por métodos propios y no por `Perform` porque aquí la respuesta importa:
//! cuántos bytes se fueron y qué no se pudo borrar. Lo que necesita root no
//! pasa por este servicio, que corre como usuario y no puede pedir contraseña.
//!
//! Los tamaños son `st_blocks`, el mismo criterio que `du`. En btrfs con
//! compresión es una cota superior: medido en este equipo, `du` da 513 GB de
//! `$HOME` en un sistema de archivos de 302 GB. Lo liberado de verdad puede ser
//! menos, y por eso la interfaz dice «hasta».

use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// Por debajo de esto un archivo no merece que alguien lo revise.
const ARCHIVO_GRANDE: u64 = 200 * 1024 * 1024;
const COMPRIMIDO_MINIMO: u64 = 1024 * 1024;
const DESCARGA_ANTIGUA: Duration = Duration::from_secs(90 * 24 * 3600);
/// La lista se pinta entera en la página: más filas no ayudan a decidir.
const MAXIMO_POR_LISTA: usize = 200;
/// Lo que se conserva del diario del sistema al limpiar. Ajustes usa este mismo
/// valor en `journalctl --vacuum-size`.
const DIARIO_CONSERVADO: u64 = 200 * 1024 * 1024;
const EXTENSIONES_COMPRIMIDO: [&str; 9] = [
    ".zip", ".7z", ".rar", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".tar.zst", ".tar.bz2",
];
const CACHES_PAQUETES: [&str; 5] = [
    "/var/cache/libdnf5",
    "/var/cache/dnf",
    "/var/cache/pacman/pkg",
    "/var/cache/apt/archives",
    "/var/cache/zypp",
];

static ESCANEANDO: AtomicBool = AtomicBool::new(false);

fn inicio() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Sin HOME".to_owned())
}
fn cache_de(inicio: &Path) -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| inicio.join(".cache"))
}
fn papelera_de(inicio: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| inicio.join(".local/share"))
        .join("Trash")
}

/// Bytes ocupados bajo `ruta` sin seguir enlaces ni cruzar a otro disco.
fn ocupado(ruta: &Path) -> u64 {
    let Ok(raiz) = std::fs::symlink_metadata(ruta) else {
        return 0;
    };
    let mut total = raiz.blocks() * 512;
    if !raiz.is_dir() {
        return total;
    }
    let mut pendientes = vec![ruta.to_path_buf()];
    while let Some(dir) = pendientes.pop() {
        let Ok(entradas) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entrada in entradas.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entrada.path()) else {
                continue;
            };
            if meta.dev() != raiz.dev() {
                continue;
            }
            total += meta.blocks() * 512;
            if meta.is_dir() {
                pendientes.push(entrada.path());
            }
        }
    }
    total
}

fn leer_kib(meminfo: &str, clave: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix(clave)?.strip_prefix(':'))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
        * 1024
}

/// `app-org.chromium.Chromium-7722.scope` → `Chromium`.
fn nombre_de_scope(scope: &str) -> Option<String> {
    let id = scope.strip_prefix("app-")?.strip_suffix(".scope")?;
    let id = id
        .rsplit_once('-')
        .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()))
        .map_or(id, |(base, _)| base);
    let nombre = id.rsplit('.').next().filter(|n| !n.is_empty())?;
    let mut letras = nombre.chars();
    let primera = letras.next()?;
    Some(primera.to_uppercase().chain(letras).collect())
}

fn memoria_de_cgroup(stat: &str) -> u64 {
    // `memory.current` incluye la caché de archivos, que el núcleo suelta solo
    // cuando hace falta: contarla atribuiría a la app memoria que no es presión.
    stat.lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(k, _)| matches!(*k, "anon" | "shmem"))
        .filter_map(|(_, v)| v.parse::<u64>().ok())
        .sum()
}

/// Memoria por app, leída de los scopes que la sesión crea al lanzarlas. Más
/// barato y más honesto que sumar `RSS` por proceso, que cuenta varias veces lo
/// compartido entre las pestañas de un navegador.
fn apps_en_memoria(app_slice: &Path) -> Vec<Value> {
    let mut apps: BTreeMap<String, u64> = BTreeMap::new();
    for entrada in std::fs::read_dir(app_slice).into_iter().flatten().flatten() {
        let scope = entrada.file_name();
        let Some(nombre) = nombre_de_scope(&scope.to_string_lossy()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entrada.path().join("memory.stat")) else {
            continue;
        };
        *apps.entry(nombre).or_default() += memoria_de_cgroup(&stat);
    }
    let mut apps: Vec<_> = apps.into_iter().filter(|(_, b)| *b > 0).collect();
    apps.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
    apps.truncate(12);
    apps.into_iter()
        .map(|(nombre, bytes)| json!({"nombre":nombre,"bytes":bytes}))
        .collect()
}

fn unidades_caidas(salida: &str) -> Vec<String> {
    salida
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// Lo instantáneo: medidores y diagnóstico. Separado de `tamanos` para que la
/// portada salga antes de recorrer ninguna carpeta.
pub async fn resumen() -> Result<Value, String> {
    let inicio = inicio()?;
    let ruta = inicio.to_string_lossy();
    let df = crate::audio::run("df", &["-B1", "--output=size,avail", &ruta]).await?;
    let mut numeros = df
        .lines()
        .nth(1)
        .ok_or("df sin datos")?
        .split_whitespace()
        .filter_map(|n| n.parse::<u64>().ok());
    let (total, libre) = (numeros.next().unwrap_or(0), numeros.next().unwrap_or(0));

    let meminfo = tokio::fs::read_to_string("/proc/meminfo")
        .await
        .map_err(|e| e.to_string())?;
    let uid = std::fs::metadata("/proc/self")
        .map_err(|e| e.to_string())?
        .uid();
    let app_slice = PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/app.slice"
    ));

    // Un fallo aquí no debe tumbar los medidores: sin systemd no hay nada que listar.
    let caidas_sistema = crate::audio::run("systemctl", &["--failed", "--no-legend", "--plain"])
        .await
        .unwrap_or_default();
    let caidas_usuario = crate::audio::run(
        "systemctl",
        &["--user", "--failed", "--no-legend", "--plain"],
    )
    .await
    .unwrap_or_default();
    let trim = crate::audio::run(
        "systemctl",
        &[
            "show",
            "fstrim.timer",
            "-p",
            "LastTriggerUSec",
            "--value",
            "--timestamp=unix",
        ],
    )
    .await
    .unwrap_or_default();

    Ok(json!({
        "disco": {"total": total, "libre": libre},
        "memoria": {
            "total": leer_kib(&meminfo, "MemTotal"),
            "disponible": leer_kib(&meminfo, "MemAvailable"),
            "swap_total": leer_kib(&meminfo, "SwapTotal"),
            "swap_libre": leer_kib(&meminfo, "SwapFree"),
        },
        "apps": tokio::task::spawn_blocking(move || apps_en_memoria(&app_slice))
            .await
            .map_err(|e| e.to_string())?,
        "diagnostico": {
            "servicios_caidos": unidades_caidas(&caidas_sistema),
            "servicios_usuario_caidos": unidades_caidas(&caidas_usuario),
            "ultimo_trim": trim.trim().trim_start_matches('@').parse::<u64>().ok(),
        },
    }))
}

/// `Archived and active journals take up 433.9M in the file system.`
fn bytes_de_diario(salida: &str) -> u64 {
    let Some(cifra) = salida
        .split_whitespace()
        .find(|p| p.starts_with(|c: char| c.is_ascii_digit()))
    else {
        return 0;
    };
    let fin = cifra
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(cifra.len());
    let numero: f64 = cifra[..fin].parse().unwrap_or(0.0);
    let escala: f64 = match cifra[fin..].chars().next() {
        Some('K') => 1024.0,
        Some('M') => 1024.0 * 1024.0,
        Some('G') => 1024.0 * 1024.0 * 1024.0,
        Some('T') => 1024.0_f64.powi(4),
        _ => 1.0,
    };
    (numero * escala) as u64
}

/// Lo que hay que recorrer pero sigue siendo rápido: cachés y papelera.
pub async fn tamanos() -> Result<Value, String> {
    let inicio = inicio()?;
    let diario = crate::audio::run("journalctl", &["--disk-usage"])
        .await
        .map(|s| bytes_de_diario(&s))
        .unwrap_or(0);
    let flatpak = crate::audio::run("flatpak", &["list", "--runtime", "--columns=application"])
        .await
        .is_ok_and(|s| !s.trim().is_empty());
    tokio::task::spawn_blocking(move || {
        let cache = cache_de(&inicio);
        let mut apps: Vec<(String, u64)> = std::fs::read_dir(&cache)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "thumbnails")
            .map(|n| {
                let bytes = ocupado(&cache.join(&n));
                (n, bytes)
            })
            .filter(|(_, b)| *b > 0)
            .collect();
        apps.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
        json!({
            "miniaturas": ocupado(&cache.join("thumbnails")),
            "papelera": ocupado(&papelera_de(&inicio).join("files")),
            "paquetes": CACHES_PAQUETES.iter().map(|r| ocupado(Path::new(r))).sum::<u64>(),
            "registros": diario.saturating_sub(DIARIO_CONSERVADO),
            "flatpak": flatpak,
            "caches": apps.into_iter()
                .map(|(nombre, bytes)| json!({"nombre":nombre,"bytes":bytes}))
                .collect::<Vec<_>>(),
        })
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Default)]
struct Hallazgos {
    grandes: Vec<(PathBuf, u64)>,
    comprimidos: Vec<(PathBuf, u64)>,
    proyectos: Vec<(PathBuf, u64)>,
}

/// `target` de Cargo y `node_modules` de npm: se reconstruyen con el manifiesto
/// que tienen al lado. Sin él, una carpeta `target` puede ser cualquier cosa.
fn es_compilado(dir: &Path) -> bool {
    let manifiesto = match dir.file_name().and_then(|n| n.to_str()) {
        Some("target") => "Cargo.toml",
        Some("node_modules") => "package.json",
        _ => return false,
    };
    dir.parent()
        .is_some_and(|padre| padre.join(manifiesto).is_file())
}

/// Un comprimido cuya carpeta extraída sigue al lado: `foo.tar.gz` y `foo/`.
fn ya_extraido(archivo: &Path) -> bool {
    let Some(nombre) = archivo.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let minusculas = nombre.to_lowercase();
    // Las extensiones compuestas tienen que probarse antes que `.tar`.
    let Some(extension) = EXTENSIONES_COMPRIMIDO
        .iter()
        .filter(|e| minusculas.ends_with(**e))
        .max_by_key(|e| e.len())
    else {
        return false;
    };
    let base = &nombre[..nombre.len() - extension.len()];
    !base.is_empty() && archivo.with_file_name(base).is_dir()
}

/// Recorre `dir` y devuelve lo que ocupa. `candidato` dice si lo que hay debajo
/// se puede sugerir: lo oculto cuenta para el total, pero proponer borrar algo de
/// `.config` o un paquete de `.git` rompería cosas sin que nadie lo entendiera.
fn recorrer(dir: &Path, disco: u64, candidato: bool, hallazgos: &mut Hallazgos) -> u64 {
    let mut total = 0;
    for entrada in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let ruta = entrada.path();
        let Ok(meta) = std::fs::symlink_metadata(&ruta) else {
            continue;
        };
        if meta.dev() != disco {
            continue;
        }
        let bytes = meta.blocks() * 512;
        let oculto = entrada.file_name().to_string_lossy().starts_with('.');
        let sugerible = candidato && !oculto;
        if meta.is_dir() {
            let compilado = sugerible && es_compilado(&ruta);
            let dentro = recorrer(&ruta, disco, sugerible && !compilado, hallazgos);
            if compilado {
                hallazgos.proyectos.push((ruta, bytes + dentro));
            }
            total += bytes + dentro;
        } else {
            if sugerible && meta.is_file() {
                if bytes >= ARCHIVO_GRANDE {
                    hallazgos.grandes.push((ruta.clone(), bytes));
                }
                if bytes >= COMPRIMIDO_MINIMO && ya_extraido(&ruta) {
                    hallazgos.comprimidos.push((ruta, bytes));
                }
            }
            total += bytes;
        }
    }
    total
}

fn lista(mut filas: Vec<(PathBuf, u64)>) -> Vec<Value> {
    filas.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
    filas.truncate(MAXIMO_POR_LISTA);
    filas
        .into_iter()
        .map(|(ruta, bytes)| json!({"ruta":ruta,"bytes":bytes}))
        .collect()
}

fn descargas_antiguas(carpeta: &Path, ahora: SystemTime) -> Vec<(PathBuf, u64)> {
    std::fs::read_dir(carpeta)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = std::fs::symlink_metadata(e.path()).ok()?;
            let edad = ahora.duration_since(meta.modified().ok()?).ok()?;
            (meta.is_file() && edad >= DESCARGA_ANTIGUA).then(|| (e.path(), meta.blocks() * 512))
        })
        .collect()
}

fn escanear_inicio(inicio: &Path, descargas: Option<&Path>) -> Result<Value, String> {
    let disco = std::fs::symlink_metadata(inicio)
        .map_err(|e| e.to_string())?
        .dev();
    let mut hallazgos = Hallazgos::default();
    let mut carpetas = Vec::new();
    for entrada in std::fs::read_dir(inicio)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let ruta = entrada.path();
        let Ok(meta) = std::fs::symlink_metadata(&ruta) else {
            continue;
        };
        if meta.dev() != disco || !meta.is_dir() {
            continue;
        }
        let nombre = entrada.file_name().to_string_lossy().into_owned();
        let bytes =
            meta.blocks() * 512 + recorrer(&ruta, disco, !nombre.starts_with('.'), &mut hallazgos);
        carpetas.push((nombre, bytes));
    }
    carpetas.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
    carpetas.truncate(10);
    let antiguas = descargas
        .map(|d| descargas_antiguas(d, SystemTime::now()))
        .unwrap_or_default();
    Ok(json!({
        "carpetas": carpetas.into_iter()
            .map(|(nombre, bytes)| json!({"nombre":nombre,"bytes":bytes}))
            .collect::<Vec<_>>(),
        "proyectos": lista(hallazgos.proyectos),
        "grandes": lista(hallazgos.grandes),
        "comprimidos": lista(hallazgos.comprimidos),
        "descargas": lista(antiguas),
    }))
}

/// Recorre todo `$HOME`: medido con 1,6 millones de archivos, 4,5 s con la caché
/// de disco caliente y 12 s un `find` en frío. Por eso va aparte, a petición, y
/// solo admite uno a la vez.
pub async fn escanear() -> Result<Value, String> {
    if ESCANEANDO.swap(true, Ordering::AcqRel) {
        return Err("Ya hay una búsqueda en curso".into());
    }
    let resultado = async {
        let inicio = inicio()?;
        let descargas = crate::audio::run("xdg-user-dir", &["DOWNLOAD"])
            .await
            .ok()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|d| *d != inicio && d.is_dir());
        tokio::task::spawn_blocking(move || escanear_inicio(&inicio, descargas.as_deref()))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    ESCANEANDO.store(false, Ordering::Release);
    resultado
}

/// Conjunto cerrado: la interfaz no puede pedir que se borre una ruta cualquiera.
#[derive(Debug, Deserialize)]
#[serde(tag = "objetivo", rename_all = "snake_case", deny_unknown_fields)]
pub enum Limpieza {
    Miniaturas {},
    Papelera {},
    CacheApp { nombre: String },
    Proyecto { ruta: String },
}

#[derive(Default)]
struct Borrado {
    bytes: u64,
    fallos: u64,
}

/// Sigue con lo demás si algo no se deja borrar: una limpieza a medias es útil,
/// siempre que quien la pidió se entere de lo que quedó.
fn borrar(ruta: &Path, cuenta: &mut Borrado) {
    let Ok(meta) = std::fs::symlink_metadata(ruta) else {
        cuenta.fallos += 1;
        return;
    };
    if meta.is_dir() {
        // Las cachés de Go y otras dejan carpetas de solo lectura, que impiden
        // borrar lo que tienen dentro aunque sean tuyas.
        if meta.permissions().mode() & 0o700 != 0o700 {
            let _ = std::fs::set_permissions(ruta, std::fs::Permissions::from_mode(0o700));
        }
        vaciar(ruta, cuenta);
        if std::fs::remove_dir(ruta).is_ok() {
            cuenta.bytes += meta.blocks() * 512;
        } else {
            cuenta.fallos += 1;
        }
    } else if std::fs::remove_file(ruta).is_ok() {
        cuenta.bytes += meta.blocks() * 512;
    } else {
        cuenta.fallos += 1;
    }
}
fn vaciar(dir: &Path, cuenta: &mut Borrado) {
    for entrada in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        borrar(&entrada.path(), cuenta);
    }
}

fn nombre_valido(nombre: &str) -> bool {
    let mut partes = Path::new(nombre).components();
    matches!(partes.next(), Some(Component::Normal(_))) && partes.next().is_none()
}

/// Solo un `target`/`node_modules` real dentro de `$HOME`, comprobado aquí y no
/// en la interfaz. `canonicalize` resuelve enlaces: un enlace llamado `target`
/// que apunte fuera no pasa.
fn proyecto_valido(ruta: &str, inicio: &Path) -> Result<PathBuf, String> {
    let real = std::fs::canonicalize(ruta).map_err(|e| e.to_string())?;
    if real.starts_with(inicio) && real != inicio && es_compilado(&real) {
        Ok(real)
    } else {
        Err("Esa carpeta no es de compilación".into())
    }
}

fn limpiar_en(limpieza: &Limpieza, inicio: &Path) -> Result<Borrado, String> {
    let mut cuenta = Borrado::default();
    match limpieza {
        Limpieza::Miniaturas {} => vaciar(&cache_de(inicio).join("thumbnails"), &mut cuenta),
        Limpieza::Papelera {} => {
            let papelera = papelera_de(inicio);
            for parte in ["files", "info", "expunged"] {
                vaciar(&papelera.join(parte), &mut cuenta);
            }
            let _ = std::fs::remove_file(papelera.join("directorysizes"));
        }
        Limpieza::CacheApp { nombre } => {
            if !nombre_valido(nombre) {
                return Err("Nombre de caché no válido".into());
            }
            borrar(&cache_de(inicio).join(nombre), &mut cuenta);
        }
        Limpieza::Proyecto { ruta } => borrar(&proyecto_valido(ruta, inicio)?, &mut cuenta),
    }
    Ok(cuenta)
}

pub async fn limpiar(carga: &str) -> Result<Value, String> {
    let limpieza: Limpieza = serde_json::from_str(carga).map_err(|e| e.to_string())?;
    let inicio = inicio()?;
    let cuenta = tokio::task::spawn_blocking(move || limpiar_en(&limpieza, &inicio))
        .await
        .map_err(|e| e.to_string())??;
    Ok(json!({"liberados":cuenta.bytes,"fallos":cuenta.fallos}))
}

/// Mover a la papelera no libera espacio, pero deja deshacerlo: es lo que toca
/// con archivos del usuario, que no se pueden regenerar.
pub async fn a_papelera(carga: &str) -> Result<Value, String> {
    let rutas: Vec<String> = serde_json::from_str(carga).map_err(|e| e.to_string())?;
    let inicio = inicio()?;
    let mut movidos = Vec::new();
    let mut fallos = 0u64;
    for ruta in rutas {
        let dentro = std::fs::canonicalize(&ruta)
            .is_ok_and(|real| real.starts_with(&inicio) && real != inicio);
        if dentro
            && crate::audio::run("gio", &["trash", "--", &ruta])
                .await
                .is_ok()
        {
            movidos.push(ruta);
        } else {
            fallos += 1;
        }
    }
    Ok(json!({"movidos":movidos,"fallos":fallos}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carpeta(nombre: &str) -> PathBuf {
        let ruta =
            std::env::temp_dir().join(format!("bookos-mant-{}-{nombre}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ruta);
        std::fs::create_dir_all(&ruta).expect("carpeta temporal escribible");
        ruta
    }

    #[test]
    fn el_scope_da_un_nombre_legible() {
        assert_eq!(
            nombre_de_scope("app-org.chromium.Chromium-7722.scope").as_deref(),
            Some("Chromium")
        );
        assert_eq!(
            nombre_de_scope("app-vesktop-9907.scope").as_deref(),
            Some("Vesktop")
        );
        assert_eq!(nombre_de_scope("bookos-system.service"), None);
    }

    #[test]
    fn la_memoria_de_una_app_no_cuenta_la_cache_de_archivos() {
        assert_eq!(memoria_de_cgroup("anon 100\nfile 900\nshmem 20\n"), 120);
    }

    #[test]
    fn lee_el_tamano_del_diario() {
        let salida = "Archived and active journals take up 433.9M in the file system.";
        assert_eq!(bytes_de_diario(salida), (433.9 * 1024.0 * 1024.0) as u64);
        assert_eq!(bytes_de_diario("Sin datos"), 0);
    }

    #[test]
    fn target_sin_manifiesto_no_es_de_compilacion() {
        let raiz = carpeta("target");
        std::fs::create_dir_all(raiz.join("a/target")).expect("crear");
        std::fs::create_dir_all(raiz.join("b/target")).expect("crear");
        std::fs::write(raiz.join("b/Cargo.toml"), "").expect("crear");
        assert!(!es_compilado(&raiz.join("a/target")));
        assert!(es_compilado(&raiz.join("b/target")));
        assert!(proyecto_valido(raiz.join("a/target").to_str().expect("utf8"), &raiz).is_err());
        assert!(proyecto_valido(raiz.join("b/target").to_str().expect("utf8"), &raiz).is_ok());
        std::fs::remove_dir_all(raiz).expect("limpiar");
    }

    #[test]
    fn un_proyecto_fuera_de_inicio_se_rechaza() {
        let raiz = carpeta("fuera");
        std::fs::create_dir_all(raiz.join("p/target")).expect("crear");
        std::fs::write(raiz.join("p/Cargo.toml"), "").expect("crear");
        std::fs::create_dir_all(raiz.join("casa")).expect("crear");
        assert!(
            proyecto_valido(
                raiz.join("p/target").to_str().expect("utf8"),
                &raiz.join("casa")
            )
            .is_err()
        );
        std::fs::remove_dir_all(raiz).expect("limpiar");
    }

    #[test]
    fn el_nombre_de_cache_no_puede_salirse_de_la_carpeta() {
        assert!(nombre_valido("uv"));
        assert!(!nombre_valido("../.ssh"));
        assert!(!nombre_valido("a/b"));
        assert!(!nombre_valido("/etc"));
        assert!(!nombre_valido(".."));
    }

    #[test]
    fn detecta_el_comprimido_ya_extraido() {
        let raiz = carpeta("comprimidos");
        std::fs::create_dir(raiz.join("foo")).expect("crear");
        assert!(ya_extraido(&raiz.join("foo.tar.gz")));
        assert!(ya_extraido(&raiz.join("foo.zip")));
        assert!(!ya_extraido(&raiz.join("bar.zip")));
        assert!(!ya_extraido(&raiz.join("foo.txt")));
        std::fs::remove_dir_all(raiz).expect("limpiar");
    }

    #[test]
    fn lo_oculto_cuenta_pero_no_se_sugiere() {
        let raiz = carpeta("oculto");
        std::fs::create_dir_all(raiz.join("p/.oculta/q/target")).expect("crear");
        std::fs::write(raiz.join("p/.oculta/q/Cargo.toml"), "x").expect("crear");
        std::fs::create_dir_all(raiz.join("p/r/target")).expect("crear");
        std::fs::write(raiz.join("p/r/Cargo.toml"), "x").expect("crear");
        let datos = escanear_inicio(&raiz, None).expect("escaneo");
        let proyectos = datos["proyectos"].as_array().expect("lista");
        assert_eq!(proyectos.len(), 1);
        assert!(
            proyectos[0]["ruta"]
                .as_str()
                .expect("ruta")
                .ends_with("p/r/target")
        );
        std::fs::remove_dir_all(raiz).expect("limpiar");
    }

    #[test]
    fn borrar_no_sigue_enlaces_y_entra_en_carpetas_de_solo_lectura() {
        let raiz = carpeta("borrar");
        let fuera = raiz.join("fuera.txt");
        std::fs::write(&fuera, "conservar").expect("crear");
        let dir = raiz.join("cache");
        std::fs::create_dir_all(dir.join("ro")).expect("crear");
        std::fs::write(dir.join("ro/a"), "x").expect("crear");
        std::os::unix::fs::symlink(&fuera, dir.join("enlace")).expect("crear");
        std::fs::set_permissions(dir.join("ro"), std::fs::Permissions::from_mode(0o500))
            .expect("permisos");
        let mut cuenta = Borrado::default();
        borrar(&dir, &mut cuenta);
        assert_eq!(cuenta.fallos, 0);
        assert!(!dir.exists());
        assert_eq!(
            std::fs::read_to_string(&fuera).expect("sigue ahí"),
            "conservar"
        );
        std::fs::remove_dir_all(raiz).expect("limpiar");
    }

    #[test]
    fn el_contrato_rechaza_objetivos_desconocidos() {
        assert!(serde_json::from_str::<Limpieza>(r#"{"objetivo":"miniaturas"}"#).is_ok());
        assert!(serde_json::from_str::<Limpieza>(r#"{"objetivo":"disco"}"#).is_err());
        assert!(
            serde_json::from_str::<Limpieza>(r#"{"objetivo":"miniaturas","ruta":"/"}"#).is_err()
        );
    }
}
