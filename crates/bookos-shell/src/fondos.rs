//! El catálogo de fondos instalados.
//!
//! Los de BookOS vienen **emparejados**: cada familia tiene una imagen clara y
//! una oscura —`blue.png` en `Light/` y `blue_dark.png` en `Dark/`—, y esa
//! pareja es la unidad que el usuario elige. Elegir un fichero suelto y no una
//! familia es lo que hace que el cambio de tema no pueda llevarse el fondo con
//! él, que era el problema de partida.
//!
//! ## De dónde salen las rutas
//!
//! Este módulo es el **único** sitio que sabe dónde viven los fondos. Antes la
//! lista estaba escrita dentro del compositor y la tarjeta de Apariencia no
//! podía verla, así que una tenía que enterarse de lo que hacía la otra.
//!
//! ## Por qué la miniatura es el SVG y no el PNG
//!
//! Medido: decodificar los cuatro PNG de 2880×1800 cuesta 152 ms —50, 52, 25 y
//! 24—, y eso es lo que tardaría en abrirse la tarjeta. Los SVG de al lado son
//! de cuatro kilobytes, media docena de degradados, y los rasteriza resvg al
//! tamaño que se le pida por el mismo camino que los iconos del dock. Una
//! familia sin SVG se queda sin miniatura, pero se puede elegir igual.

use std::path::{Path, PathBuf};

/// Una pareja de fondos, la unidad que se elige.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Familia {
    /// El nombre del fichero sin extensión ni sufijo: `blue`, `ember`…
    pub nombre: String,
    pub claro: PathBuf,
    pub oscuro: PathBuf,
    /// El SVG con el que se dibuja la miniatura, del tema que toque. `None` si
    /// esta familia no trae vectorial.
    pub vista: Option<PathBuf>,
}

/// Las carpetas donde se buscan los fondos, en orden de preferencia.
///
/// Lo del usuario antes que lo del sistema, y el repositorio de wallpapers tal
/// cual se clona para desarrollar en medio: sin él, trabajar en esto obligaría
/// a instalarlos en cada cambio.
// 0.1.0
fn bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        bases.push(home.join(".local/share/wallpapers/BookOS"));
        bases.push(home.join("Descargas/BookOS/BookOS-Wallpapers/Wallpapers-0.6"));
    }
    bases.push(PathBuf::from("/usr/share/wallpapers/BookOS"));
    bases
}

/// El nombre del fichero oscuro de una familia.
// 0.1.0
fn oscuro_de(nombre: &str, ext: &str) -> String {
    format!("{nombre}_dark.{ext}")
}

/// Busca un fichero de la familia en una base, mirando primero la carpeta del
/// tema y luego la raíz.
///
/// La raíz se sigue mirando por las instalaciones hechas con la versión
/// anterior del instalador, que volcaba las dos carpetas juntas.
// 0.1.0
fn en(base: &Path, carpeta: &str, archivo: &str) -> Option<PathBuf> {
    let candidatos = [base.join(carpeta).join(archivo), base.join(archivo)];
    candidatos.into_iter().find(|p| p.is_file())
}

/// Las familias completas que hay instaladas, sin repetir nombre.
///
/// Una familia solo cuenta si tiene **las dos** imágenes del mismo formato:
/// media pareja no sirve para lo que existe esto. PNG va primero para que los
/// SVG que hoy acompañan a los fondos como previsualización no los sustituyan
/// según el orden arbitrario del directorio. Se devuelven ordenadas por nombre
/// para que los iconos no bailen entre sesiones.
// 0.1.0
pub fn instaladas(claro: bool) -> Vec<Familia> {
    let mut familias: Vec<Familia> = Vec::new();
    for base in bases() {
        for formato in ["png", "webp", "jpg", "jpeg", "svg"] {
            let Ok(entradas) = std::fs::read_dir(base.join("Light")) else {
                continue;
            };
            for entrada in entradas.flatten() {
                let ruta = entrada.path();
                if ruta.extension().and_then(|e| e.to_str()) != Some(formato) {
                    continue;
                }
                let Some(nombre) = ruta.file_stem().and_then(|n| n.to_str()) else {
                    continue;
                };
                // Una base más preferente manda; dentro de ella, el primer
                // formato de la tabla. Así la elección no depende de readdir.
                if familias.iter().any(|f| f.nombre == nombre) {
                    continue;
                }
                let Some(oscuro) = en(&base, "Dark", &oscuro_de(nombre, formato)) else {
                    tracing::debug!(nombre, formato, "fondo sin pareja oscura; no se ofrece");
                    continue;
                };
                let vista = if claro {
                    en(&base, "Light", &format!("{nombre}.svg"))
                } else {
                    en(&base, "Dark", &oscuro_de(nombre, "svg"))
                };
                familias.push(Familia {
                    nombre: nombre.to_string(),
                    claro: ruta,
                    oscuro,
                    vista,
                });
            }
        }
    }
    familias.sort_by(|a, b| a.nombre.cmp(&b.nombre));
    familias
}

/// El tono dominante de una familia, en grados del círculo cromático.
///
/// Sale del **SVG**, no de la imagen: decodificar los PNG de 2880×1800 para
/// esto costaría medio segundo al abrir la tarjeta, y hay un test que lo
/// prohíbe (`abrir_apariencia_no_decodifica_los_fondos`). Del vectorial basta
/// con leer la cabecera y quedarse con el color más vivo que declare.
///
/// Solo cuentan las paradas de los degradados (`stop-color`), que es lo que se
/// ve. Los SVG exportados traen debajo de todo un `<rect fill="#103042">`, el
/// fondo del marco, tapado entero por el primer `<path>`: contando los `fill`,
/// ese azul ganaba a los grises cálidos de `beige_alice`, que salía a 202° en
/// vez de 34°. En los otros cuatro el resultado es el mismo con y sin `fill`.
///
/// «Más vivo» es saturación por cercanía al gris medio: sin el segundo factor
/// gana un `#031E10` casi negro, que en tono es verde pero no es el color con
/// el que nadie reconoce el fondo. Medido contra los cuatro que trae BookOS:
/// beige_alice 34°, blue 210°, ember 14°, pine 126°, purple 276°, que es lo que
/// se ve.
///
/// `None` si la familia no trae vectorial o no declara ningún color.
// 0.1.0
pub fn tono(familia: &Familia) -> Option<f32> {
    // 4 KiB llegan: en los fondos de BookOS las paradas empiezan antes del byte
    // 3100, detrás de las curvas. En `beige_alice` claro las últimas quedan
    // fuera y sale 39° en vez de 34°, que sigue siendo su color.
    let vista = familia.vista.as_ref()?;
    use std::io::Read as _;
    let fichero = std::fs::File::open(vista).ok()?;
    let mut cabecera = Vec::new();
    fichero.take(4096).read_to_end(&mut cabecera).ok()?;
    let texto = String::from_utf8_lossy(&cabecera);

    let mut mejor: Option<(f32, f32)> = None;
    for trozo in texto.split("stop-color=\"#").skip(1) {
        let hex = trozo.as_bytes();
        if hex.len() < 6 || !hex[..6].iter().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let canal = |i: usize| u8::from_str_radix(&trozo[i..i + 2], 16).map(|v| v as f32 / 255.0);
        let (Ok(r), Ok(g), Ok(b)) = (canal(0), canal(2), canal(4)) else {
            continue;
        };
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let luz = (max + min) / 2.0;
        let croma = max - min;
        if croma <= f32::EPSILON {
            continue;
        }
        let saturacion = croma / (1.0 - (luz * 2.0 - 1.0).abs()).max(f32::EPSILON);
        let peso = saturacion * (1.0 - (luz * 2.0 - 1.0).abs());
        if mejor.is_some_and(|(m, _)| peso <= m) {
            continue;
        }
        let h = if max == r {
            60.0 * (((g - b) / croma) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / croma + 2.0)
        } else {
            60.0 * ((r - g) / croma + 4.0)
        };
        mejor = Some((peso, (h + 360.0) % 360.0));
    }
    mejor.map(|(_, h)| h)
}

/// Cuánto se parecen dos tonos, en grados: de 0 (el mismo) a 180 (opuestos).
// 0.1.0
pub fn distancia_de_tono(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

/// La familia que está puesta, global del proceso.
///
/// Va aquí y no como argumento por lo mismo que el tema y el acento: la tarjeta
/// de Apariencia se construye desde una tabla de punteros a función y no recibe
/// la configuración. Es un `Mutex` y no un atómico porque es un nombre, no un
/// número; se toca al arrancar y al elegir, o sea nunca en un bucle de dibujo.
static ELEGIDA: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// El nombre de familia de una ruta de fondo: `blue_dark.png` es `blue`.
///
/// Devuelve `None` para lo que no tenga nombre de fichero. Un fondo que el
/// usuario haya puesto a mano y no pertenezca a ninguna familia instalada da un
/// nombre que no está en el catálogo, y entonces no se marca ninguna miniatura,
/// que es exactamente lo que hay que enseñar.
// 0.1.0
pub fn familia_de(ruta: &Path) -> Option<String> {
    let tallo = ruta.file_stem()?.to_str()?;
    Some(tallo.strip_suffix("_dark").unwrap_or(tallo).to_string())
}

/// Guarda qué familia está puesta. `None` cuando no hay ninguna elegida.
// 0.1.0
pub fn poner_elegida(nombre: Option<String>) {
    if let Ok(mut guard) = ELEGIDA.lock() {
        *guard = nombre;
    }
}

// 0.1.0
pub fn elegida() -> Option<String> {
    ELEGIDA.lock().ok().and_then(|g| g.clone())
}

/// La familia por defecto, la que se pone cuando nadie ha elegido ninguna.
pub const POR_DEFECTO: &str = "blue";

/// La pareja que toca sin configuración: la familia por defecto si está, y si
/// no, la primera que haya.
// 0.1.0
pub fn por_defecto(claro: bool) -> Option<Familia> {
    let familias = instaladas(claro);
    familias
        .iter()
        .find(|f| f.nombre == POR_DEFECTO)
        .or_else(|| familias.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El nombre del fichero oscuro sale del claro, y no al revés: es lo que
    /// permite recorrer `Light/` y emparejar.
    #[test]
    fn la_pareja_oscura_se_deduce_del_nombre() {
        assert_eq!(oscuro_de("blue", "png"), "blue_dark.png");
        assert_eq!(oscuro_de("pine", "svg"), "pine_dark.svg");
    }

    /// Y al revés: de cualquiera de las dos rutas se saca la familia, que es lo
    /// que permite marcar la miniatura correcta con el tema oscuro puesto.
    #[test]
    fn la_familia_sale_de_cualquiera_de_las_dos_rutas() {
        let f = |r: &str| familia_de(Path::new(r));
        assert_eq!(f("/x/Light/blue.png").as_deref(), Some("blue"));
        assert_eq!(f("/x/Dark/blue_dark.png").as_deref(), Some("blue"));
        // Uno puesto a mano que no es de ninguna familia: da su propio nombre,
        // que no estará en el catálogo y no marcará nada.
        assert_eq!(f("/home/yo/foto.jpg").as_deref(), Some("foto"));
        assert_eq!(f("/"), None);
    }

    /// El tono sale del SVG y acierta con los fondos que trae BookOS.
    ///
    /// Los grados están medidos sobre los ficheros de `Wallpapers-0.6`. El
    /// margen es de 8°, por debajo de la distancia más corta entre dos
    /// familias —beige_alice↔ember, 20°— y a la vez tolera que alguien retoque
    /// un degradado sin romper la prueba.
    #[test]
    fn el_tono_de_cada_familia_es_el_que_se_ve() {
        let esperados = [
            ("beige_alice", 34.0),
            ("blue", 210.0),
            ("ember", 14.0),
            ("pine", 126.0),
            ("purple", 276.0),
        ];
        let familias = instaladas(false);
        if familias.is_empty() {
            return;
        }
        for (nombre, grados) in esperados {
            let Some(f) = familias.iter().find(|f| f.nombre == nombre) else {
                continue;
            };
            let Some(t) = tono(f) else {
                panic!("{nombre} no dio tono");
            };
            let d = distancia_de_tono(t, grados);
            assert!(d < 8.0, "{nombre}: {t:.0}° contra los {grados:.0}° medidos");
        }
    }

    #[test]
    fn las_familias_vienen_completas_y_ordenadas() {
        let familias = instaladas(true);
        if familias.is_empty() {
            return;
        }
        for f in &familias {
            assert!(f.claro.is_file(), "{} sin imagen clara", f.nombre);
            assert!(f.oscuro.is_file(), "{} sin imagen oscura", f.nombre);
        }
        let mut nombres: Vec<&str> = familias.iter().map(|f| f.nombre.as_str()).collect();
        let ordenados = {
            let mut c = nombres.clone();
            c.sort();
            c
        };
        assert_eq!(nombres, ordenados, "las familias no salen ordenadas");
        nombres.dedup();
        assert_eq!(nombres.len(), familias.len(), "hay familias repetidas");
    }
}
