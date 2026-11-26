use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use bookos_shell::Battery;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idioma {
    Es,
    En,
}

impl Idioma {
    pub const TODOS: [Self; 2] = [Self::Es, Self::En];

    // 0.1.1
    pub fn detectar() -> Self {
        let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .or_else(|| leer_clave_conf("/etc/default/bookos-greeter", "LANG"))
            .or_else(|| leer_clave_conf("/etc/locale.conf", "LANG"))
            .unwrap_or_else(|| "es_ES.UTF-8".into());
        if lang.trim().starts_with("en") {
            Self::En
        } else {
            Self::Es
        }
    }

    // 0.1.1
    pub fn locale(self) -> &'static str {
        match self {
            Self::Es => "es_ES.UTF-8",
            Self::En => "en_US.UTF-8",
        }
    }

    // 0.1.1
    pub fn etiqueta(self) -> &'static str {
        match self {
            Self::Es => "Español (ES)",
            Self::En => "English (US)",
        }
    }

    // 0.1.1
    pub fn t<'a>(self, es: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Es => es,
            Self::En => en,
        }
    }

    // 0.1.1
    pub fn fecha_hoy(self) -> String {
        let hoy = bookos_shell::Fecha::hoy();
        match self {
            Self::Es => {
                const DIAS: [&str; 7] = [
                    "lunes",
                    "martes",
                    "miércoles",
                    "jueves",
                    "viernes",
                    "sábado",
                    "domingo",
                ];
                const MESES: [&str; 12] = [
                    "enero",
                    "febrero",
                    "marzo",
                    "abril",
                    "mayo",
                    "junio",
                    "julio",
                    "agosto",
                    "septiembre",
                    "octubre",
                    "noviembre",
                    "diciembre",
                ];
                format!(
                    "{}, {} de {}",
                    DIAS[hoy.dia_semana() as usize],
                    hoy.dia,
                    MESES[(hoy.mes - 1) as usize]
                )
            }
            Self::En => {
                const DAYS: [&str; 7] = [
                    "Monday",
                    "Tuesday",
                    "Wednesday",
                    "Thursday",
                    "Friday",
                    "Saturday",
                    "Sunday",
                ];
                const MONTHS: [&str; 12] = [
                    "January",
                    "February",
                    "March",
                    "April",
                    "May",
                    "June",
                    "July",
                    "August",
                    "September",
                    "October",
                    "November",
                    "December",
                ];
                format!(
                    "{}, {} {}",
                    DAYS[hoy.dia_semana() as usize],
                    MONTHS[(hoy.mes - 1) as usize],
                    hoy.dia
                )
            }
        }
    }
}

pub const DISTRIBUCIONES: &[(&str, &str, &str)] = &[
    ("es", "Español (España)", "Spanish (Spain)"),
    ("latam", "Español (Latinoamérica)", "Spanish (Latin America)"),
    ("us", "Inglés (EE. UU.)", "English (US)"),
    ("gb", "Inglés (Reino Unido)", "English (UK)"),
    ("fr", "Francés", "French"),
    ("de", "Alemán", "German"),
    ("pt", "Portugués", "Portuguese"),
    ("it", "Italiano", "Italian"),
];

// 0.1.1
pub fn teclado_inicial() -> String {
    std::env::var("XKB_DEFAULT_LAYOUT")
        .ok()
        .or_else(|| leer_clave_conf("/etc/default/bookos-greeter", "XKB_DEFAULT_LAYOUT"))
        .or_else(|| leer_clave_conf("/etc/vconsole.conf", "KEYMAP"))
        .and_then(|d| Some(d.split(',').next()?.trim().to_lowercase()))
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "es".into())
}

// 0.1.1
fn leer_clave_conf(ruta: &str, clave: &str) -> Option<String> {
    let texto = std::fs::read_to_string(ruta).ok()?;
    texto.lines().find_map(|linea| {
        let (k, v) = linea.trim().split_once('=')?;
        (k.trim() == clave).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

// 0.1.1
pub fn leer_ultimo_usuario() -> Option<String> {
    if let Ok(valor) = std::env::var("BOOKOS_LAST_USER") {
        let limpio = valor.trim();
        return (!limpio.is_empty() && cuenta_valida(limpio)).then(|| limpio.to_string());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    leer_ultimo_usuario_en(&home.join("greeter.env"))
}

pub(crate) fn leer_ultimo_usuario_en(ruta: &Path) -> Option<String> {
    let valor = leer_clave_conf(ruta.to_str()?, "BOOKOS_LAST_USER")?;
    let limpio = valor.trim();
    (!limpio.is_empty() && cuenta_valida(limpio)).then(|| limpio.to_string())
}

fn cuenta_valida(cuenta: &str) -> bool {
    cuenta
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

// 0.1.1
pub fn guardar_preferencias_greeter(
    layout: &str,
    lang: &str,
    ultimo_usuario: Option<&str>,
) -> Option<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    guardar_preferencias_en(&home.join("greeter.env"), layout, lang, ultimo_usuario)
}

pub(crate) fn guardar_preferencias_en(
    ruta: &Path,
    layout: &str,
    lang: &str,
    ultimo_usuario: Option<&str>,
) -> Option<()> {
    let temporal = ruta.with_extension("parcial");
    let ultimo = ultimo_usuario
        .map(str::trim)
        .filter(|u| !u.is_empty() && cuenta_valida(u))
        .map(str::to_string)
        .or_else(|| leer_clave_conf(ruta.to_str()?, "BOOKOS_LAST_USER").filter(|u| cuenta_valida(u)));
    let mut contenido = format!("XKB_DEFAULT_LAYOUT={layout}\nLANG={lang}\n");
    if let Some(u) = ultimo {
        contenido.push_str(&format!("BOOKOS_LAST_USER={u}\n"));
    }
    std::fs::write(&temporal, contenido).ok()?;
    let _ = std::fs::set_permissions(&temporal, std::fs::Permissions::from_mode(0o644));
    std::fs::rename(&temporal, ruta).ok()
}

// 0.1.1
pub fn aplicar_teclado_sistema(layout: &str) {
    let _ = Command::new("busctl")
        .args([
            "--system",
            "call",
            "org.freedesktop.locale1",
            "/org/freedesktop/locale1",
            "org.freedesktop.locale1",
            "SetVConsoleKeyboard",
            "ssbb",
            layout,
            "",
            "true",
            "false",
        ])
        .output();
}

// 0.1.1
pub fn aplicar_idioma_sistema(locale: &str) {
    let arg = format!("LANG={locale}");
    let _ = Command::new("busctl")
        .args([
            "--system",
            "call",
            "org.freedesktop.locale1",
            "/org/freedesktop/locale1",
            "org.freedesktop.locale1",
            "SetLocale",
            "asb",
            "1",
            &arg,
            "false",
        ])
        .output();
}

/// Traduce una tecla física según la distribución elegida en el greeter cuando
/// difiere de la distribución con la que arrancó Cage (`base`).
///
/// Como Cage fija el mapa XKB al iniciar, cambiar de distribución en vivo sin
/// cerrar la sesión de bienvenida traduce los caracteres imprimibles más
/// habituales entre `es`/`latam` y `us`/`gb` (y `fr`/`de` para letras).
// 0.1.1
pub fn traducir_caracter(texto: &str, base: &str, destino: &str, shift: bool) -> Option<String> {
    if base == destino || texto.chars().count() != 1 {
        return None;
    }
    let ch = texto.chars().next()?;
    let mut c = ch;
    // Primero llevamos de `base` a posición física QWERTY US.
    if matches!(base, "es" | "latam") {
        c = de_es_a_us(c, shift);
    } else if base == "fr" {
        c = de_fr_a_us(c);
    } else if base == "de" {
        c = de_de_a_us(c);
    }
    // Y de posición física US a `destino`.
    if matches!(destino, "es" | "latam") {
        c = de_us_a_es(c, shift);
    } else if destino == "fr" {
        c = de_us_a_fr(c);
    } else if destino == "de" {
        c = de_us_a_de(c);
    }
    (c != ch).then(|| c.to_string())
}

// 0.1.1
fn de_es_a_us(c: char, _shift: bool) -> char {
    match c {
        'ñ' => ';',
        'Ñ' => ':',
        'ç' => '\\',
        'Ç' => '|',
        '¡' => '=',
        '¿' => '+',
        '\'' => '-',
        '?' => '_',
        'º' => '`',
        'ª' => '~',
        '`' => '[',
        '^' => '{',
        '+' => ']',
        '*' => '}',
        '´' => '\'',
        '¨' => '"',
        '-' => '/',
        '_' => '?',
        '.' => '.',
        ':' => '>',
        ',' => ',',
        ';' => '<',
        '<' => '\\',
        '>' => '|',
        '"' => '@',
        '·' => '#',
        '&' => '^',
        '/' => '&',
        '(' => '*',
        ')' => '(',
        '=' => ')',
        _ => c,
    }
}

// 0.1.1
fn de_us_a_es(c: char, _shift: bool) -> char {
    match c {
        ';' => 'ñ',
        ':' => 'Ñ',
        '\\' => 'ç',
        '|' => 'Ç',
        '=' => '¡',
        '+' => '¿',
        '-' => '\'',
        '_' => '?',
        '`' => 'º',
        '~' => 'ª',
        '[' => '`',
        '{' => '^',
        ']' => '+',
        '}' => '*',
        '\'' => '´',
        '"' => '¨',
        '/' => '-',
        '?' => '_',
        '>' => ':',
        '<' => ';',
        '@' => '"',
        '#' => '·',
        '^' => '&',
        '&' => '/',
        '*' => '(',
        '(' => ')',
        ')' => '=',
        _ => c,
    }
}

// 0.1.1
fn de_fr_a_us(c: char) -> char {
    match c {
        'a' => 'q',
        'A' => 'Q',
        'q' => 'a',
        'Q' => 'A',
        'z' => 'w',
        'Z' => 'W',
        'w' => 'z',
        'W' => 'Z',
        'm' => ';',
        'M' => ':',
        _ => c,
    }
}

// 0.1.1
fn de_us_a_fr(c: char) -> char {
    match c {
        'q' => 'a',
        'Q' => 'A',
        'a' => 'q',
        'A' => 'Q',
        'w' => 'z',
        'W' => 'Z',
        'z' => 'w',
        'Z' => 'W',
        ';' => 'm',
        ':' => 'M',
        _ => c,
    }
}

// 0.1.1
fn de_de_a_us(c: char) -> char {
    match c {
        'z' => 'y',
        'Z' => 'Y',
        'y' => 'z',
        'Y' => 'Z',
        _ => c,
    }
}

// 0.1.1
fn de_us_a_de(c: char) -> char {
    match c {
        'y' => 'z',
        'Y' => 'Z',
        'z' => 'y',
        'Z' => 'Y',
        _ => c,
    }
}

// ── Red Wi-Fi (nmcli) ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedWifi {
    pub ssid: String,
    pub senal: u8,
    pub activa: bool,
    pub segura: bool,
    pub guardada: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EstadoWifi {
    pub encendida: bool,
    pub cable: bool,
    pub ssid_actual: Option<String>,
    pub redes: Vec<RedWifi>,
    pub error: Option<String>,
}

impl Default for EstadoWifi {
    // 0.1.1
    fn default() -> Self {
        Self {
            encendida: true,
            cable: false,
            ssid_actual: None,
            redes: Vec::new(),
            error: None,
        }
    }
}

// 0.1.1
pub fn leer_wifi(rescan: bool) -> EstadoWifi {
    let radio = Command::new("nmcli")
        .args(["-t", "-f", "WIFI", "general"])
        .env("LC_ALL", "C")
        .output();
    let Ok(radio) = radio else {
        return EstadoWifi {
            encendida: false,
            cable: false,
            ssid_actual: None,
            redes: Vec::new(),
            error: Some("NetworkManager no disponible".into()),
        };
    };
    if !radio.status.success() {
        return EstadoWifi {
            encendida: false,
            cable: false,
            ssid_actual: None,
            redes: Vec::new(),
            error: Some(String::from_utf8_lossy(&radio.stderr).trim().to_string()),
        };
    }
    let encendida = String::from_utf8_lossy(&radio.stdout)
        .trim()
        .eq_ignore_ascii_case("enabled");

    // Perfiles guardados para saber si hace falta pedir contraseña.
    let guardadas: std::collections::HashSet<String> = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,NAME", "connection", "show"])
        .env("LC_ALL", "C")
        .output()
        .ok()
        .filter(|s| s.status.success())
        .map(|s| {
            String::from_utf8_lossy(&s.stdout)
                .lines()
                .filter_map(|l| {
                    let (tipo, nombre) = l.split_once(':')?;
                    (tipo == "802-11-wireless").then(|| desescapar_nmcli(nombre))
                })
                .collect()
        })
        .unwrap_or_default();

    let cable = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE", "device"])
        .env("LC_ALL", "C")
        .output()
        .ok()
        .filter(|s| s.status.success())
        .is_some_and(|s| {
            String::from_utf8_lossy(&s.stdout)
                .lines()
                .any(|l| l.starts_with("ethernet:connected"))
        });

    if !encendida {
        return EstadoWifi {
            encendida: false,
            cable,
            ssid_actual: None,
            redes: Vec::new(),
            error: None,
        };
    }

    let arg_rescan = if rescan { "yes" } else { "no" };
    let lista = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "IN-USE,SSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "--rescan",
            arg_rescan,
        ])
        .env("LC_ALL", "C")
        .output();

    let Ok(lista) = lista else {
        return EstadoWifi {
            encendida,
            cable,
            ssid_actual: None,
            redes: Vec::new(),
            error: None,
        };
    };

    let texto = String::from_utf8_lossy(&lista.stdout);
    let (redes, ssid_actual) = parsear_lista_wifi(&texto, &guardadas);
    EstadoWifi {
        encendida,
        cable,
        ssid_actual,
        redes,
        error: None,
    }
}

// 0.1.1
pub fn parsear_lista_wifi(
    texto: &str,
    guardadas: &std::collections::HashSet<String>,
) -> (Vec<RedWifi>, Option<String>) {
    let mut redes = Vec::new();
    let mut ssid_actual = None;
    for linea in texto.lines() {
        let campos = partir_nmcli(linea);
        if campos.len() < 4 {
            continue;
        }
        let activa = campos[0].trim() == "*";
        let ssid = campos[1].trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let senal = campos[2].trim().parse::<u8>().unwrap_or(0);
        let seguridad = campos[3].trim();
        let segura = !seguridad.is_empty() && seguridad != "--";
        if activa {
            ssid_actual = Some(ssid.clone());
        }
        let guardada = activa || guardadas.contains(&ssid);
        redes.push(RedWifi {
            ssid,
            senal,
            activa,
            segura,
            guardada,
        });
    }
    redes.sort_by_key(|r| {
        (
            std::cmp::Reverse(r.activa),
            std::cmp::Reverse(r.guardada),
            std::cmp::Reverse(r.senal),
        )
    });
    let mut vistos = std::collections::HashSet::new();
    redes.retain(|r| vistos.insert(r.ssid.clone()));
    redes.truncate(6);
    (redes, ssid_actual)
}

// 0.1.1
fn partir_nmcli(linea: &str) -> Vec<String> {
    let mut partes = Vec::new();
    let mut actual = String::new();
    let mut escapado = false;
    for c in linea.chars() {
        if escapado {
            actual.push(c);
            escapado = false;
        } else if c == '\\' {
            escapado = true;
        } else if c == ':' {
            partes.push(std::mem::take(&mut actual));
        } else {
            actual.push(c);
        }
    }
    partes.push(actual);
    partes
}

// 0.1.1
fn desescapar_nmcli(texto: &str) -> String {
    partir_nmcli(texto).join(":")
}

// 0.1.1
pub fn alternar_wifi(encender: bool) -> Result<EstadoWifi, String> {
    let arg = if encender { "on" } else { "off" };
    let salida = Command::new("nmcli")
        .args(["radio", "wifi", arg])
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| e.to_string())?;
    if !salida.status.success() {
        return Err(String::from_utf8_lossy(&salida.stderr).trim().to_string());
    }
    Ok(leer_wifi(false))
}

// 0.1.1
pub fn conectar_wifi(
    ssid: &str,
    password: Option<&str>,
    idioma: Idioma,
) -> Result<EstadoWifi, String> {
    let mut cmd = Command::new("nmcli");
    cmd.env("LC_ALL", "C")
        .args(["-w", "25", "device", "wifi", "connect", ssid]);
    if let Some(clave) = password.filter(|p| !p.is_empty()) {
        cmd.args(["password", clave]);
    }
    let salida = cmd.output().map_err(|e| e.to_string())?;
    if !salida.status.success() {
        let err = String::from_utf8_lossy(&salida.stderr).trim().to_string();
        let out = String::from_utf8_lossy(&salida.stdout).trim().to_string();
        return Err(if !err.is_empty() {
            err
        } else if !out.is_empty() {
            out
        } else {
            idioma
                .t("No se pudo conectar a la red", "Couldn't connect to the network")
                .into()
        });
    }
    Ok(leer_wifi(false))
}

// 0.1.1
pub fn desconectar_wifi() -> Result<EstadoWifi, String> {
    let dev = Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE,STATE", "device"])
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&dev.stdout);
    if let Some(iface) = texto.lines().find_map(|l| {
        let mut p = l.split(':');
        let d = p.next()?;
        let t = p.next()?;
        let s = p.next()?;
        (t == "wifi" && s == "connected").then_some(d)
    }) {
        let _ = Command::new("nmcli")
            .args(["device", "disconnect", iface])
            .env("LC_ALL", "C")
            .output();
    }
    Ok(leer_wifi(false))
}

// ── Batería y perfiles de energía ────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EstadoBateria {
    pub dato: Option<Battery>,
    pub tope: Option<u8>,
    pub perfil: String,
    pub perfiles_disponibles: Vec<String>,
}

fn leer_primera_ruta(rutas: &[&str]) -> Option<String> {
    rutas
        .iter()
        .find_map(|ruta| std::fs::read_to_string(ruta).ok())
}

// 0.1.1
pub fn leer_bateria(con_minutos: bool) -> EstadoBateria {
    let dato = Battery::read(con_minutos);
    let tope = leer_tope_carga();
    let silencio_adaptativo = std::fs::read_to_string("/run/bookos-termico/estado")
        .is_ok_and(|s| s.trim() == "balanced-quiet");
    let perfil = leer_primera_ruta(&[
        "/sys/firmware/acpi/platform_profile",
        "/sys/class/platform-profile/platform-profile-0/profile",
    ])
    .map(|s| s.trim().to_string())
    .map(|s| {
        if s == "low-power" || s == "quiet" {
            if silencio_adaptativo {
                "balanced".to_string()
            } else {
                "power-saver".to_string()
            }
        } else if s == "balanced-performance" {
            "balanced".to_string()
        } else {
            s
        }
    })
    .unwrap_or_else(|| "balanced".into());
    let perfiles_disponibles = leer_primera_ruta(&[
        "/sys/firmware/acpi/platform_profile_choices",
        "/sys/class/platform-profile/platform-profile-0/choices",
    ])
    .map(|s| {
        let mut v = Vec::new();
        for token in s.split_whitespace() {
            let norm = match token {
                "low-power" | "quiet" | "power-saver" => "power-saver",
                "balanced" | "balanced-performance" => "balanced",
                "performance" => "performance",
                _ => continue,
            };
            if !v.contains(&norm.to_string()) {
                v.push(norm.to_string());
            }
        }
        v
    })
    .filter(|v| !v.is_empty())
    .unwrap_or_else(|| {
        vec![
            "power-saver".into(),
            "balanced".into(),
            "performance".into(),
        ]
    });
    EstadoBateria {
        dato,
        tope,
        perfil,
        perfiles_disponibles,
    }
}

// 0.1.1
fn leer_tope_carga() -> Option<u8> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.flatten() {
        let p = e.path();
        if p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("BAT"))
            && let Ok(s) = std::fs::read_to_string(p.join("charge_control_end_threshold"))
            && let Ok(n) = s.trim().parse::<u8>()
            && (1..100).contains(&n)
        {
            return Some(n);
        }
    }
    None
}

// 0.1.1
pub fn cambiar_perfil_energia(perfil: &str) {
    let _ = Command::new("busctl")
        .args([
            "--system",
            "set-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "ActiveProfile",
            "s",
            perfil,
        ])
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_redes_wifi_con_dos_puntos_escapados_y_ordena_por_senal() {
        let salida = "*:Mi\\:Red\\:Casa:84:WPA2\n :Vecino:92:WPA2\n :Abierta:40:--\n :Mi\\:Red\\:Casa:50:WPA2\n";
        let guardadas = std::collections::HashSet::new();
        let (redes, actual) = parsear_lista_wifi(salida, &guardadas);
        assert_eq!(actual.as_deref(), Some("Mi:Red:Casa"));
        assert_eq!(redes.len(), 3);
        assert_eq!(redes[0].ssid, "Mi:Red:Casa");
        assert!(redes[0].activa);
        assert!(redes[0].segura);
        assert_eq!(redes[1].ssid, "Vecino");
        assert_eq!(redes[1].senal, 92);
        assert_eq!(redes[2].ssid, "Abierta");
        assert!(!redes[2].segura);
    }

    #[test]
    fn traduce_teclas_entre_es_y_us() {
        assert_eq!(
            traducir_caracter(";", "es", "us", false).as_deref(),
            Some("<")
        );
        assert_eq!(
            traducir_caracter("ñ", "es", "us", false).as_deref(),
            Some(";")
        );
        assert_eq!(
            traducir_caracter(";", "us", "es", false).as_deref(),
            Some("ñ")
        );
        assert_eq!(traducir_caracter("a", "es", "es", false), None);
    }
}
