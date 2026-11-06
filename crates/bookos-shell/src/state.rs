//! Los datos que enseña el panel.
//!
//! Todo se lee de sysfs y de la hora local; sin D-Bus, sin daemons y sin
//! esperas. Que el panel no dependa de ningún servicio es parte de por qué
//! puede estar pintado en el primer frame de la sesión.
//!
//! Eso también marca **qué** puede enseñar el panel: batería, red y brillo
//! están en sysfs y entran; el volumen y lo que suena no lo están, y llegarán
//! cuando haya con quién hablar (PipeWire, MPRIS), no leyendo ficheros a
//! ciegas. Ninguna lectura de aquí bloquea, y cada una se rinde en cuanto algo
//! no está donde debería: en una máquina sin batería, sin red o sin
//! retroiluminación el estado simplemente no se dibuja.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelData {
    /// Hora local ya formateada, "HH:MM".
    pub clock: String,
    pub battery: Option<Battery>,
    pub network: Option<Network>,
    /// Brillo de la pantalla en tanto por ciento, si hay retroiluminación.
    pub brightness: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// Enchufado a la corriente. No es lo mismo que `charging`: con el umbral
    /// de carga alcanzado el portátil está enchufado y **no** carga.
    pub plugged: bool,
    /// Minutos que quedan — de autonomía si descarga, para llegar al tope de
    /// carga si carga. `None` cuando el driver no da corriente o vale cero.
    pub minutes: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    pub kind: Link,
    /// El enlace está levantado y con portadora.
    pub up: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Wifi,
    Cable,
}

impl PanelData {
    // 0.1.0
    pub fn read() -> Self {
        Self {
            clock: local_hhmm(),
            battery: Battery::read(true),
            network: Network::read(),
            brightness: read_brightness(),
        }
    }
}

impl Battery {
    /// `con_minutos` decide si además se estima el tiempo restante, que es la
    /// parte cara: exige `current_now`, y eso es una consulta al controlador
    /// embebido —0,53 ms medidos— frente a los 0,02 de `capacity`. Quien llama
    /// decide con qué cadencia la quiere; ver `widgets::bateria::CADA_MINUTOS`.
    // 0.1.0
    // modified 0.1.1 -- pública para otros crates
    pub fn read(con_minutos: bool) -> Option<Self> {
        let path = battery_path()?;
        let percent = read_num(path.join("capacity"))?.min(100) as u8;
        let status = fs::read_to_string(path.join("status")).ok()?;
        let status = status.trim();
        // Manda el adaptador, y la batería solo si no hay ninguno. Su `status`
        // llega tarde: el driver lo sirve de una caché de 1 s
        // (`battery.cache_time`) y el aviso de udev del adaptador llega antes
        // de que cambie. Leyéndolo de ahí, al desenchufar seguía diciendo
        // «Charging» y no sonaba nada; al volver a enchufar decía por fin
        // «Discharging» y sonaba la desconexión, y 0,77 s después la conexión
        // (visto en el registro de la sesión). Leer `online` cuesta 0,035 ms.
        //
        // "Not charging" es lo que dice el driver cuando está enchufado pero
        // parado en el umbral de carga: no está cargando, pero tampoco
        // gastando batería.
        let plugged = ac_online().unwrap_or(matches!(status, "Charging" | "Not charging" | "Full"));
        // Sin corriente no se carga, diga lo que diga un `status` atrasado.
        let charging = plugged && status == "Charging";
        Some(Self {
            percent,
            charging,
            plugged,
            minutes: con_minutos
                .then(|| {
                    // Envenenado solo si otro hilo reventó a mitad de una
                    // suma: la media que dejó sigue siendo un número válido.
                    let mut media = FLUJO.lock().unwrap_or_else(|e| e.into_inner());
                    remaining_minutes(&path, charging, &mut media)
                })
                .flatten(),
        })
    }
}

/// Minutos restantes: lo que falta por gastar o por cargar, entre la
/// corriente **media** de los últimos minutos.
///
/// El kernel expone la batería en carga (`charge_*`, µAh) o en energía
/// (`energy_*`, µWh) según el driver; las dos parejas dividen igual, así que
/// basta con encontrar la que exista.
///
/// Cargando, la meta es el tope de carga y no `charge_full`: con el umbral al
/// 80 % la cuenta hasta el lleno daba 0:52 donde faltaban 0:30 (medido aquí al
/// 53 %, 2,2 A).
// 0.1.0
fn remaining_minutes(path: &Path, charging: bool, media: &mut Flujo) -> Option<u32> {
    // El `exists()` se repite en cada lectura a propósito: recordarlo en un
    // global ataría la respuesta a la primera batería que se mirase, y lo que
    // cuesta aquí es `current_now` —0,53 ms medidos, una consulta al
    // controlador embebido— no este `stat`, que son 0,02.
    let (ahora, tope, flujo) = if path.join("charge_now").exists() {
        ("charge_now", "charge_full", "current_now")
    } else {
        ("energy_now", "energy_full", "power_now")
    };

    let ahora = read_num(path.join(ahora))? as f32;
    let flujo = media.poner(
        read_num(path.join(flujo))? as f32,
        charging,
        std::time::Instant::now(),
    )?;
    let restante = if charging {
        let lleno = read_num(path.join(tope))? as f32;
        let meta = charge_limit_en(path).map_or(lleno, |u| lleno * f32::from(u) / 100.0);
        meta - ahora
    } else {
        ahora
    };
    let minutos = restante * 60.0 / flujo;
    // Con la carga frenando justo antes del tope la corriente cae a unos pocos
    // mA y la cuenta da días: eso no es una estimación, es ruido.
    (restante > 0.0 && minutos < 24.0 * 60.0).then_some(minutos as u32)
}

/// La corriente media.
///
/// Se promedia el **flujo** y no los minutos, como se hacía antes: los minutos
/// son su inverso y su media no es la de nada, y sobre todo un valor de minutos
/// promediado no avanza solo. Con la corriente media y la carga leída en cada
/// momento, la cuenta baja minuto a minuto aunque el consumo no cambie.
///
/// Hay una sola para todo el proceso (ver [`FLUJO`]): el panel, el emergente de
/// energía y el buscador leen la misma batería y tienen que decir el mismo
/// tiempo. Con una media por cada uno, el emergente recién abierto enseñaba la
/// lectura cruda y el panel su media, y no coincidían nunca.
///
/// La media es exponencial en el **tiempo** —constante de [`Flujo::TAU`]—, no
/// en el número de lecturas: quien más lee no pesa más. La corriente
/// instantánea en este portátil va de 1:28 de autonomía compilando a 8:18 en
/// reposo treinta segundos después; con dos minutos, un pico de medio minuto
/// mueve la media un 22 % y no la triplica.
#[derive(Default)]
struct Flujo(Option<(bool, f32, std::time::Instant)>);

static FLUJO: std::sync::Mutex<Flujo> = std::sync::Mutex::new(Flujo(None));

impl Flujo {
    const TAU: f32 = 120.0;

    // 0.1.0
    fn poner(
        &mut self,
        instantaneo: f32,
        charging: bool,
        ahora: std::time::Instant,
    ) -> Option<f32> {
        let nuevo = match self.0 {
            // Enchufar o desenchufar cambia lo que se mide: lo de antes no vale.
            Some((cargando, previo, cuando)) if cargando == charging => {
                let pasado = ahora.saturating_duration_since(cuando).as_secs_f32();
                previo + (instantaneo - previo) * (1.0 - (-pasado / Self::TAU).exp())
            }
            _ => instantaneo,
        };
        self.0 = Some((charging, nuevo, ahora));
        (nuevo > 0.0).then_some(nuevo)
    }
}

/// El tope de carga, si el portátil tiene uno puesto. Es la explicación de por
/// qué una batería enchufada se queda parada en el 80 % y no llega al 100.
// 0.1.0
pub(crate) fn charge_limit() -> Option<u8> {
    charge_limit_en(&battery_path()?)
}

// 0.1.0
fn charge_limit_en(path: &Path) -> Option<u8> {
    read_num(path.join("charge_control_end_threshold"))
        .filter(|u| *u > 0 && *u < 100)
        .map(|u| u as u8)
}

// 0.1.0
fn ac_online() -> Option<bool> {
    let dir = fs::read_dir("/sys/class/power_supply").ok()?;
    for entrada in dir.filter_map(|e| e.ok()) {
        let path = entrada.path();
        if fs::read_to_string(path.join("type")).is_ok_and(|t| t.trim() == "Mains") {
            return Some(read_num(path.join("online")) == Some(1));
        }
    }
    None
}

impl Network {
    /// Primera interfaz de verdad que esté levantada; si no hay ninguna, la
    /// primera que exista, para poder enseñar "sin red" en vez de nada.
    ///
    /// `lo` se descarta siempre: el bucle local está arriba hasta en una
    /// máquina desconectada, así que contarlo haría que el panel jurase que
    /// hay red cuando no la hay.
    // 0.1.0
    // modified 0.1.1 -- pública para otros crates
    pub fn read() -> Option<Self> {
        let mut interfaces: Vec<PathBuf> = fs::read_dir("/sys/class/net")
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some("lo"))
            .collect();
        interfaces.sort();

        let mut caida = None;
        for path in interfaces {
            let kind = if path.join("wireless").is_dir() {
                Link::Wifi
            } else {
                Link::Cable
            };
            // `operstate` es lo que dice el driver; `carrier` es si hay cable o
            // asociación. Se piden los dos porque una interfaz puede estar
            // "up" administrativamente con el cable fuera.
            let up = fs::read_to_string(path.join("operstate")).is_ok_and(|s| s.trim() == "up")
                && read_num(path.join("carrier")) == Some(1);
            if up {
                return Some(Self { kind, up });
            }
            caida.get_or_insert(Self { kind, up: false });
        }
        caida
    }
}

/// Dispositivo válido de retroiluminación. Se prefiere el controlador nativo
/// a las interfaces de compatibilidad del firmware.
// 0.1.0
pub(crate) fn backlight_device() -> Option<String> {
    backlight_path()?
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// El valor crudo máximo del dispositivo. En este panel es 400: mandarle un
/// tanto por ciento directamente lo dejaría al 25 % de lo pedido.
// 0.1.0
pub(crate) fn backlight_max(dispositivo: &str) -> Option<u32> {
    read_num(
        PathBuf::from("/sys/class/backlight")
            .join(dispositivo)
            .join("max_brightness"),
    )
}

/// Recuerda el resultado de buscar un dispositivo en sysfs.
///
/// Se usa para la batería. La retroiluminación se redetecta por separado para
/// tolerar retiradas y cambios de controlador durante la sesión.
///
/// **El fallo no se cachea.** Si no hay nada, se vuelve a mirar la próxima vez:
/// un módulo puede cargarse después de arrancar la sesión, y recordar «aquí no
/// hay batería» dejaría el panel sin ella hasta reiniciar. Buscar y no
/// encontrar es justo el caso barato —un `readdir` de un directorio vacío—, así
/// que reintentarlo no cuesta.
// 0.1.0
fn recordar(
    cache: &std::sync::OnceLock<PathBuf>,
    buscar: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(ruta) = cache.get() {
        return Some(ruta.clone());
    }
    let encontrada = buscar()?;
    // Si dos hilos llegan a la vez gana el primero; los dos devuelven una ruta
    // válida, que es lo único que importa.
    let _ = cache.set(encontrada.clone());
    Some(encontrada)
}

// 0.1.0
fn backlight_path() -> Option<PathBuf> {
    elegir_luz(Path::new("/sys/class/backlight"), false)
}

// No se conserva una ruta entre refrescos: el controlador puede reaparecer
// tras suspender o cargarse después del arranque.
// 0.1.0
fn elegir_luz(root: &Path, teclado: bool) -> Option<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            !teclado
                || p.file_name()
                    .is_some_and(|n| n.to_string_lossy().contains("kbd_backlight"))
        })
        .filter(|p| {
            read_num(p.join("max_brightness")).is_some_and(|n| n > 0)
                && read_num(p.join("brightness")).is_some()
        })
        .collect();
    paths.sort_by_key(|p| {
        let kind = fs::read_to_string(p.join("type")).unwrap_or_default();
        (
            match kind.trim() {
                "raw" => 0,
                "platform" => 1,
                _ => 2,
            },
            p.clone(),
        )
    });
    paths.into_iter().next()
}

// 0.1.0
pub(crate) fn teclado_actual() -> Option<(String, u32, u32)> {
    let path = elegir_luz(Path::new("/sys/class/leds"), true)?;
    let max = read_num(path.join("max_brightness"))?;
    Some((
        path.file_name()?.to_str()?.into(),
        read_num(path.join("brightness"))?.min(max),
        max,
    ))
}

// 0.1.0
pub(crate) fn read_brightness() -> Option<u8> {
    let path = backlight_path()?;
    let max = read_num(path.join("max_brightness"))?;
    if max == 0 {
        return None;
    }
    // `brightness` y no `actual_brightness`: **medido en esta máquina, leer
    // `actual_brightness` cuesta 0,512 ms y `brightness` 0,030**, diecisiete
    // veces menos. La diferencia es que el primero le pregunta al hardware por
    // el valor efectivo y el segundo devuelve el que se pidió, y en un panel de
    // portátil son el mismo número salvo durante la rampa de una transición.
    // Ese medio milisegundo se pagaba en **cada** evento de udev, que llegan en
    // ráfaga al mover el brillo con la tecla.
    let ahora = read_num(path.join("brightness"))?;
    Some((u64::from(ahora.min(max)) * 100 / u64::from(max)) as u8)
}

// 0.1.0
fn battery_path() -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    recordar(&CACHE, || {
        let mut candidates: Vec<PathBuf> = fs::read_dir("/sys/class/power_supply")
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("BAT"))
            })
            .collect();
        candidates.sort();
        candidates.into_iter().next()
    })
}

// 0.1.0
fn read_num(path: impl AsRef<Path>) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Hora local en "HH:MM", sin dependencias.
///
/// El desfase respecto a UTC se saca de `localtime(3)` a través de la única vía
/// que no arrastra una librería de zonas horarias entera: preguntarle a la libc
/// del sistema. `localtime_r` es reentrante, así que no pisa el estado global.
// 0.1.0
pub(crate) fn local_hhmm() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (h, m) = local_hm(now);
    format!("{h:02}:{m:02}")
}

/// Una fecha del calendario local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fecha {
    pub anio: i32,
    /// 1..=12.
    pub mes: u32,
    /// 1..=31.
    pub dia: u32,
}

impl Fecha {
    /// Hoy, según la zona horaria del sistema.
    // 0.1.0
    pub fn hoy() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let tm = local_tm(secs);
        Self {
            // `tm_year` cuenta desde 1900 y `tm_mon` desde 0. Es la fuente
            // clásica de errores de "un mes de menos" con `struct tm`.
            anio: tm.2 + 1900,
            mes: tm.3 as u32 + 1,
            dia: tm.4 as u32,
        }
    }

    /// Cuántos días tiene un mes, con el año bisiesto del calendario gregoriano.
    // 0.1.0
    pub fn dias_del_mes(anio: i32, mes: u32) -> u32 {
        match mes {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if bisiesto(anio) => 29,
            2 => 28,
            _ => 30,
        }
    }

    /// Qué día de la semana cae, con **0 = lunes**.
    ///
    /// Es la convención del calendario de BookOS (la cabecera empieza en Lun) y
    /// no la de `tm_wday`, que empieza en domingo. Se calcula con la fórmula de
    /// Sakamoto en vez de preguntarle a la libc: así una fecha cualquiera del
    /// mes que se está mirando no necesita convertirse a tiempo Unix, que con
    /// los cambios de hora es donde aparecen los días que bailan.
    // 0.1.0
    pub fn dia_semana(&self) -> u32 {
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let mut y = self.anio;
        if self.mes < 3 {
            y -= 1;
        }
        let dom0 = (y + y / 4 - y / 100 + y / 400 + T[(self.mes - 1) as usize] + self.dia as i32)
            .rem_euclid(7);
        // Sakamoto da 0 = domingo; aquí la semana empieza en lunes.
        (dom0 as u32 + 6) % 7
    }
}

// 0.1.0
fn bisiesto(anio: i32) -> bool {
    (anio % 4 == 0 && anio % 100 != 0) || anio % 400 == 0
}

#[cfg(unix)]
// 0.1.0
pub(crate) fn local_hm(unix_secs: i64) -> (u8, u8) {
    let tm = local_tm(unix_secs);
    (tm.0, tm.1)
}

/// La hora local de ahora mismo, en `(hora, minuto)`.
///
/// La usa el tema automático para saber si toca claro u oscuro. Va aquí, con
/// el resto del calendario, porque es donde vive la única llamada a
/// `localtime_r` del proyecto y no tiene sentido tener dos.
// 0.1.0
pub fn hora_local_ahora() -> (u8, u8) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    local_hm(secs)
}

/// `(hora, minuto, años desde 1900, mes desde 0, día del mes)` en local.
#[cfg(unix)]
// 0.1.0
fn local_tm(unix_secs: i64) -> (u8, u8, i32, i32, i32) {
    // Declaración mínima de libc en vez de la dependencia entera: solo se
    // necesita `localtime_r` y los campos de `tm` que usa el reloj.
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const i8,
    }
    unsafe extern "C" {
        // 0.1.0
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }

    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: `localtime_r` escribe en el `tm` que le damos y no guarda el
    // puntero. Si falla devuelve null y nos quedamos con el tm a cero.
    let ok = unsafe { !localtime_r(&unix_secs, &mut tm).is_null() };
    if ok {
        (
            tm.tm_hour as u8,
            tm.tm_min as u8,
            tm.tm_year,
            tm.tm_mon,
            tm.tm_mday,
        )
    } else {
        // Un tm a cero sería el año 1900 y el día 0, que no existe. Mejor una
        // fecha válida aunque sea falsa: el calendario se dibuja igual.
        (0, 0, 70, 0, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luz_descarta_controladores_invalidos_y_redetecta_tras_retirada() {
        let root = bateria_falsa("luces", &[]);
        let crear = |nombre: &str, tipo: &str, maximo: &str| {
            let p = root.join(nombre);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("type"), tipo).unwrap();
            fs::write(p.join("max_brightness"), maximo).unwrap();
            fs::write(p.join("brightness"), "0").unwrap();
            p
        };
        crear("a_invalido", "raw", "0");
        let firmware = crear("acpi_video0", "firmware", "10");
        let raw = crear("intel_backlight", "raw", "400");
        assert_eq!(elegir_luz(&root, false), Some(raw.clone()));
        fs::remove_dir_all(raw).unwrap();
        assert_eq!(elegir_luz(&root, false), Some(firmware));
        let nuevo = crear("amdgpu_bl0", "raw", "255");
        assert_eq!(elegir_luz(&root, false), Some(nuevo));
    }

    #[test]
    fn teclado_apagado_se_detecta_y_un_led_roto_no_oculta_el_valido() {
        let root = bateria_falsa("leds", &[]);
        fs::create_dir(root.join("a::kbd_backlight")).unwrap();
        let teclado = root.join("samsung-galaxybook::kbd_backlight");
        fs::create_dir(&teclado).unwrap();
        fs::write(teclado.join("max_brightness"), "3").unwrap();
        fs::write(teclado.join("brightness"), "0").unwrap();
        assert_eq!(elegir_luz(&root, true), Some(teclado.clone()));
        fs::remove_dir_all(teclado).unwrap();
        assert_eq!(elegir_luz(&root, true), None);
    }

    /// Monta un directorio con la pinta de una batería de sysfs.
    fn bateria_falsa(nombre: &str, ficheros: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bookos-bat-{nombre}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (fichero, valor) in ficheros {
            fs::write(dir.join(fichero), valor).unwrap();
        }
        dir
    }

    #[test]
    fn descargando_divide_carga_entre_corriente() {
        // 3.075 Ah restantes a 0,523 A → 5 h 52 min.
        let dir = bateria_falsa(
            "descarga",
            &[
                ("charge_now", "3075000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "523000\n"),
            ],
        );
        assert_eq!(
            remaining_minutes(&dir, false, &mut Flujo::default()),
            Some(352)
        );
    }

    #[test]
    fn cargando_cuenta_lo_que_falta_para_llenarse() {
        // Faltan 1.025 Ah de 4.1; a 2 A, media hora larga.
        let dir = bateria_falsa(
            "carga",
            &[
                ("charge_now", "3075000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "2000000\n"),
            ],
        );
        assert_eq!(
            remaining_minutes(&dir, true, &mut Flujo::default()),
            Some(30)
        );
    }

    /// Con tope de carga, cargando se cuenta hasta el tope: al 53 % con el
    /// umbral en 80 y 2,2 A faltan 30 minutos, no los 52 de llegar al lleno.
    #[test]
    fn cargando_con_tope_cuenta_hasta_el_tope() {
        let dir = bateria_falsa(
            "tope",
            &[
                ("charge_now", "2173000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "2196000\n"),
                ("charge_control_end_threshold", "80\n"),
            ],
        );
        assert_eq!(
            remaining_minutes(&dir, true, &mut Flujo::default()),
            Some(30)
        );
    }

    /// Ya en el tope, aunque el driver aún diga «Charging», no hay tiempo.
    #[test]
    fn en_el_tope_no_hay_tiempo_de_carga() {
        let dir = bateria_falsa(
            "en-tope",
            &[
                ("charge_now", "3300000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "40000\n"),
                ("charge_control_end_threshold", "80\n"),
            ],
        );
        assert_eq!(remaining_minutes(&dir, true, &mut Flujo::default()), None);
    }

    /// Un pico de medio minuto mueve la media una quinta parte, no entera; y
    /// enchufar la descarta.
    #[test]
    fn la_media_del_flujo_va_por_tiempo() {
        let t0 = std::time::Instant::now();
        let mut f = Flujo::default();
        assert_eq!(f.poner(500.0, false, t0), Some(500.0));
        let pico = f
            .poner(1500.0, false, t0 + std::time::Duration::from_secs(30))
            .expect("flujo positivo");
        assert!((700.0..730.0).contains(&pico), "media tras el pico: {pico}");
        // Una segunda lectura en el mismo instante no pesa nada.
        let repetida = f
            .poner(1500.0, false, t0 + std::time::Duration::from_secs(30))
            .expect("flujo positivo");
        assert_eq!(repetida, pico);
        assert_eq!(
            f.poner(2000.0, true, t0 + std::time::Duration::from_secs(31)),
            Some(2000.0)
        );
    }

    /// Algunos drivers dan la batería en energía (µWh) y no en carga (µAh).
    #[test]
    fn tambien_sirve_la_pareja_de_energia() {
        let dir = bateria_falsa(
            "energia",
            &[
                ("energy_now", "30000000\n"),
                ("energy_full", "50000000\n"),
                ("power_now", "10000000\n"),
            ],
        );
        assert_eq!(
            remaining_minutes(&dir, false, &mut Flujo::default()),
            Some(180)
        );
    }

    /// Corriente cero es lo que se lee justo al enchufar o con la carga
    /// parada en el umbral. Dividir ahí sería un panic, y el panel tiene que
    /// enseñar el porcentaje igual: por eso es `None` y no un cero.
    #[test]
    fn corriente_cero_no_da_tiempo() {
        let dir = bateria_falsa(
            "parada",
            &[
                ("charge_now", "3075000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "0\n"),
            ],
        );
        assert_eq!(remaining_minutes(&dir, false, &mut Flujo::default()), None);
    }

    #[test]
    fn sin_ficheros_no_hay_estimacion() {
        let dir = bateria_falsa("vacia", &[]);
        assert_eq!(remaining_minutes(&dir, false, &mut Flujo::default()), None);
    }
}
