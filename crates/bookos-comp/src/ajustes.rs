//! Puente entre BookOS Settings y el compositor: la interfaz `org.bookos.Desktop`.
//!
//! Un único servicio en el bus de sesión para todo lo que Settings necesita del
//! escritorio. Empezó con la recarga de la pantalla de bloqueo y ahora lleva
//! también la configuración de pantallas, que es lo que sustituye a KScreen
//! cuando la sesión es BookOS y no Plasma.
//!
//! **Contrato** (`org.bookos.Desktop`, en `/org/bookos/Desktop`):
//!
//! ```text
//! ReloadConfig(s seccion) -> b
//!     Relee `panel.conf`. Secciones: "lockscreen"/"bloqueo",
//!     "activities"/"actividades", "keys"/"teclas" (este, `teclas.conf`),
//!     "brightness"/"brillo", "all". "channel"/"canal" no lee `panel.conf`
//!     sino `bookos-update.conf`: el color del logo según el canal.
//!
//! GetCapabilities() -> a{sv}
//!     Qué sabe hacer este backend. Diccionario a propósito: añadir una
//!     capacidad no rompe a quien ya lee las que conoce. Claves de la v1:
//!     version(u) backend(s) live_apply(b) fractional_scale(b)
//!     per_output_scale(b) position(b) rotation(b) mode(b) refresh(b) vrr(b)
//!     multi_output(b) primary(b) hdr(b) icc(b) night_light(b) escalas(ad)
//!     ambient_light(b).
//!
//! GetConfig() -> s / ApplyConfig(s json) -> (b ok, s error)
//!     Configuración estructurada y versionada. Settings edita JSON y nunca
//!     analiza ni reescribe `panel.conf`; la validación y persistencia viven
//!     en este proceso.
//!
//!     `tema` es lo que el usuario eligió —claro, oscuro o automatico— y
//!     `tema_efectivo` el color que está pintado ahora mismo, ya resuelto.
//!     El segundo es de solo lectura: `ApplyConfig` lo ignora, así que se
//!     puede devolver el objeto entero tal como vino.
//!
//! GetScreensaverPreview(s estilo) -> s
//!     Miniatura SVG generada por el mismo modelo del compositor. Sin
//!     animación cuando el usuario ha elegido efectos reducidos.
//!
//! PreviewScreensaver(s estilo) -> (b ok, s error)
//!     Prueba el salvapantallas real sin guardar preferencias. Cualquier
//!     entrada lo retira tras el margen para soltar el clic de apertura.
//!
//! GetOutputs() -> a(ssssuubba(uuubb)dadiisbbbii)
//!     El censo de salidas. Ver `pantallas::Salida` para el orden de campos;
//!     la firma la genera zvariant y no hay que escribirla a mano.
//!
//! ApplyOutputConfig(a(sbuuudiisbb) config) -> (b ok, s error)
//!     Aplica en vivo y persiste. `ok=false` con el motivo en `error` cuando la
//!     validación o el hardware la rechazan; entonces **no** se guarda nada y
//!     se vuelve a lo que había.
//!
//! GetKeyRemaps() -> s / ApplyKeyRemaps(s json) -> (b ok, s error)
//!     Los remapeos de `teclas.conf` como `[{"origen": s, "destino": s}]`,
//!     con los mismos textos que el fichero. Aplicar valida, escribe y
//!     recarga; si una entrada no vale no se escribe nada.
//!
//! CaptureKey(b armar) -> b
//!     Con `true`, la siguiente tecla que no sea un modificador no llega a
//!     nadie y sale por `KeyCaptured`. No es la respuesta del método porque
//!     esperar a una persona dentro de él ocupa el ejecutor de zbus y para
//!     el resto del bus. Con `false` se desarma: quien se canse de esperar
//!     tiene que desarmar, o la tecla se tragaría más tarde.
//!
//! signal OutputsChanged()
//!     Algo cambió en las pantallas —se aplicó una configuración, se enchufó o
//!     se quitó un monitor—. Quien la reciba vuelve a pedir `GetOutputs`.
//!
//! signal KeyCaptured(u evdev, b copilot)
//!     La tecla capturada. `copilot` si era el acorde Meta+Mayús+F23.
//! ```
//!
//! **Por qué el hilo de D-Bus no toca nada gráfico.** zbus atiende el bus en su
//! propio hilo. Leer el censo es leer un `Mutex` que el compositor rellena;
//! aplicar es mandar la petición por el canal de calloop y esperar la respuesta
//! por un canal de vuelta. Ni el renderer ni el backend DRM se comparten entre
//! hilos, que es lo que permite que todo esto no lleve ni un `unsafe`.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use smithay::reexports::calloop::channel::Sender;

use crate::pantallas::{Compartido, Peticion, Salida};

pub const NOMBRE: &str = "org.bookos.Desktop";
pub const RUTA: &str = "/org/bookos/Desktop";

/// Cuánto espera el hilo de D-Bus a que el compositor conteste. Un modeset con
/// su reintento tarda decenas de milisegundos; cinco segundos es "el compositor
/// no va a contestar", no "está tardando".
const ESPERA: Duration = Duration::from_secs(5);

fn config_json(config: &bookos_shell::Config) -> serde_json::Value {
    let modo = match config.modo_tema {
        bookos_shell::tema::ModoTema::Claro => "claro",
        bookos_shell::tema::ModoTema::Oscuro => "oscuro",
        bookos_shell::tema::ModoTema::Automatico => "automatico",
    };
    let efectos = match config.efectos {
        bookos_shell::Efectos::Completos => "completos",
        bookos_shell::Efectos::Reducidos => "reducidos",
    };
    let dock: Vec<_> = config
        .dock
        .iter()
        .map(|l| {
            serde_json::json!({
                "exec": l.exec,
                "etiqueta": l.etiqueta,
                "icono": l.icono,
                "app_id": l.app_id,
            })
        })
        .collect();
    let mut json = serde_json::json!({
        "version": 1,
        "centro": config.centro,
        "derecha": config.derecha,
        "dock": dock,
        "dock_tamano": config.dock_tamano,
        "bloqueo_huella": config.bloqueo_huella,
        "escala": config.escala,
        "cursor": config.cursor,
        "fondo": config.fondo,
        "fondo_claro": config.fondo_claro,
        "fondo_oscuro": config.fondo_oscuro,
        "teclado": config.teclado,
        "escritorios": config.escritorios,
        "nombres_escritorios": config.nombres_escritorios,
        "tema": modo,
        // El color que está pintado AHORA, ya resuelto. `tema` es la
        // preferencia, y con «automatico» un cliente no puede saber de qué
        // color pintarse sin rehacer aquí el cálculo de las horas —que es lo
        // que hacía Settings, con su propio `date +%H:%M` y su propia
        // comparación—. Solo lectura: `ApplyConfig` lo ignora.
        "tema_efectivo": match bookos_shell::tema::actual() {
            bookos_shell::tema::Tema::Claro => "claro",
            bookos_shell::tema::Tema::Oscuro => "oscuro",
        },
        "tema_claro_desde": format!("{:02}:{:02}", config.tema_claro_desde.0, config.tema_claro_desde.1),
        "tema_oscuro_desde": format!("{:02}:{:02}", config.tema_oscuro_desde.0, config.tema_oscuro_desde.1),
        "acento": config.acento.nombre(),
        "efectos": efectos,
        "alto_contraste": config.alto_contraste,
        "brillo_automatico_pantalla": config.brillo_automatico.pantalla,
        "brillo_automatico_teclado": config.brillo_automatico.teclado,
        "avatar": config.avatar,
        "bloqueo_animaciones": config.bloqueo.animaciones,
        "bloqueo_fecha": config.bloqueo.fecha,
        "bloqueo_medios": config.bloqueo.medios,
        "bloqueo_reloj_y": config.bloqueo.reloj_y,
        "bloqueo_acceso_y": config.bloqueo.acceso_y,
        "bloqueo_medios_y": config.bloqueo.medios_y,
        "bloqueo_reloj_tamano": config.bloqueo.reloj_tamano,
        "bloqueo_avatar_tamano": config.bloqueo.avatar_tamano,
        "bloqueo_inactividad": config.bloqueo_inactividad,
        "suspension_inactividad": config.suspension_inactividad,
        "actividades": config.actividades.habilitadas,
        "actividades_animaciones": config.actividades.animaciones,
        "temporizador_siempre_visible": config.actividades.temporizador_siempre,
        "velocidad_touchpad": config.entrada.velocidad_touchpad,
        "velocidad_raton": config.entrada.velocidad_raton,
        "toque_para_clic": config.entrada.toque_para_clic,
        "scroll_natural": config.entrada.scroll_natural,
    });
    json["salvapantallas_inactividad"] = config.salvapantallas_inactividad.into();
    json["atenuacion_inactividad"] = config.atenuacion_inactividad.into();
    json["salvapantallas_estilo"] = config.salvapantallas_estilo.nombre().into();
    json["apagado_inactividad"] = config.apagado_inactividad.into();
    // Fuera del `json!`: con una clave más, el macro pasa del límite de
    // recursión de rustc.
    json["sonido_volumen"] = config.sonido_volumen.into();
    json["sonidos_interfaz"] = config.sonidos_interfaz.into();
    json["genio"] = config.genio.into();
    json["teclado_retardo"] = config.teclado_retardo.into();
    json["teclado_velocidad"] = config.teclado_velocidad.into();
    json["volumen_paso"] = config.pasos.volumen.into();
    json["volumen_paso_mantener"] = config.pasos.volumen_mantener.into();
    json["brillo_paso"] = config.pasos.brillo.into();
    json["brillo_paso_mantener"] = config.pasos.brillo_mantener.into();
    json["bloqueo_nombre"] = config.bloqueo.nombre.into();
    json["bloqueo_energia"] = config.bloqueo.energia.into();
    json["dock_esquiva"] = config.dock_esquiva.into();
    json["pantalla_tactil"] = config.entrada.pantalla_tactil.into();
    json["panel_esquiva"] = config.panel_esquiva.into();
    json["bloqueo_reloj_fuente"] = config.bloqueo.reloj_fuente.nombre().into();
    json["bloqueo_reloj_grosor"] = config.bloqueo.reloj_grosor.into();
    // Solo lectura: los estilos que se pueden elegir porque su fuente está
    // instalada. Ajustes ofrece estos y no la lista entera; la ISO solo
    // garantiza Noto Sans y Nunito.
    json["bloqueo_reloj_fuentes"] = bookos_shell::bloqueo::estilos_instalados()
        .into_iter()
        .map(|f| serde_json::json!({ "nombre": f.nombre(), "familia": f.familia() }))
        .collect::<Vec<_>>()
        .into();
    let luz = config.luz_nocturna;
    json["luz_nocturna"] = match luz.modo {
        bookos_shell::ModoLuzNocturna::Apagada => "no",
        bookos_shell::ModoLuzNocturna::Encendida => "si",
        bookos_shell::ModoLuzNocturna::Programada => "programada",
    }
    .into();
    json["luz_nocturna_temperatura"] = luz.temperatura.into();
    json["luz_nocturna_desde"] = format!("{:02}:{:02}", luz.desde.0, luz.desde.1).into();
    json["luz_nocturna_hasta"] = format!("{:02}:{:02}", luz.hasta.0, luz.hasta.1).into();
    // Lo que se ve ahora, con el horario y lo encendido a mano ya resueltos.
    // Solo lectura, como `tema_efectivo`.
    json["luz_nocturna_activa"] = bookos_shell::luz_nocturna_encendida().into();
    json
}

fn validar_config_json(json: &str) -> Result<Vec<(String, String)>, String> {
    let valor: serde_json::Value =
        serde_json::from_str(json).map_err(|err| format!("JSON no válido: {err}"))?;
    let objeto = valor
        .as_object()
        .ok_or_else(|| "la configuración debe ser un objeto JSON".to_string())?;
    if let Some(version) = objeto.get("version")
        && version.as_u64() != Some(1)
    {
        return Err("versión de configuración no compatible".into());
    }

    let texto = |v: &serde_json::Value, clave: &str| {
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("«{clave}» debe ser texto"))
    };
    let booleano = |v: &serde_json::Value, clave: &str| {
        v.as_bool()
            .map(|v| {
                if v {
                    "si".to_string()
                } else {
                    "no".to_string()
                }
            })
            .ok_or_else(|| format!("«{clave}» debe ser booleano"))
    };
    let numero = |v: &serde_json::Value, clave: &str, min: f64, max: f64| {
        let n = v
            .as_f64()
            .filter(|n| n.is_finite() && (min..=max).contains(n))
            .ok_or_else(|| format!("«{clave}» debe estar entre {min} y {max}"))?;
        Ok::<_, String>(n.to_string())
    };
    let entero = |v: &serde_json::Value, clave: &str, min: u64, max: u64| {
        let n = v
            .as_u64()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("«{clave}» debe ser un entero entre {min} y {max}"))?;
        Ok::<_, String>(n.to_string())
    };
    let seguro = |s: &str, clave: &str, separadores: &[char]| {
        if s.len() > 1024 || s.contains(['\n', '\r']) || s.contains(separadores) {
            Err(format!("«{clave}» contiene separadores no permitidos"))
        } else {
            Ok(s.to_string())
        }
    };

    let mut salida = Vec::new();
    for (clave, valor) in objeto {
        if clave == "version" {
            continue;
        }
        let guardado = match clave.as_str() {
            "centro" => {
                if valor.is_null() {
                    String::new()
                } else {
                    seguro(&texto(valor, clave)?, clave, &[','])?
                }
            }
            "fondo" | "fondo_claro" | "fondo_oscuro" | "avatar" => {
                if valor.is_null() {
                    String::new()
                } else {
                    seguro(&texto(valor, clave)?, clave, &[])?
                }
            }
            "teclado" => {
                if valor.is_null() {
                    String::new()
                } else {
                    let v = texto(valor, clave)?;
                    if v.len() > 64
                        || !v
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_-+,".contains(c))
                    {
                        return Err("«teclado» contiene caracteres no permitidos".into());
                    }
                    v
                }
            }
            "derecha" | "nombres_escritorios" => {
                let items = valor
                    .as_array()
                    .ok_or_else(|| format!("«{clave}» debe ser una lista"))?;
                let max = if clave == "nombres_escritorios" {
                    5
                } else {
                    32
                };
                if items.len() > max {
                    return Err(format!("«{clave}» tiene demasiados elementos"));
                }
                items
                    .iter()
                    .map(|v| texto(v, clave).and_then(|s| seguro(&s, clave, &[','])))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            }
            "dock" => {
                let items = valor
                    .as_array()
                    .ok_or_else(|| "«dock» debe ser una lista".to_string())?;
                if items.len() > 64 {
                    return Err("«dock» tiene demasiados lanzadores".into());
                }
                items
                    .iter()
                    .map(|item| {
                        let o = item
                            .as_object()
                            .ok_or_else(|| "cada lanzador debe ser un objeto".to_string())?;
                        let campo = |n: &str| {
                            o.get(n)
                                .ok_or_else(|| format!("falta dock.{n}"))
                                .and_then(|v| texto(v, n))
                                .and_then(|s| seguro(&s, n, &[':', ',']))
                        };
                        Ok(format!(
                            "{}:{}:{}:{}",
                            campo("exec")?,
                            campo("etiqueta")?,
                            campo("icono")?,
                            campo("app_id")?
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .join(", ")
            }
            "escala" => {
                if valor.is_null() {
                    String::new()
                } else {
                    numero(valor, clave, 0.5, 4.0)?
                }
            }
            "cursor" => entero(valor, clave, 8, 128)?,
            "escritorios" => entero(valor, clave, 1, 5)?,
            "bloqueo_inactividad"
            | "salvapantallas_inactividad"
            | "suspension_inactividad"
            | "atenuacion_inactividad"
            | "apagado_inactividad" => entero(valor, clave, 0, 86_400)?,
            "velocidad_touchpad" | "velocidad_raton" => numero(valor, clave, -1.0, 1.0)?,
            "bloqueo_reloj_y" => numero(valor, clave, 0.02, 0.40)?,
            "bloqueo_acceso_y" => numero(valor, clave, 0.18, 0.72)?,
            "bloqueo_medios_y" => numero(valor, clave, 0.42, 0.88)?,
            "bloqueo_reloj_tamano" => numero(valor, clave, 72.0, 220.0)?,
            "bloqueo_avatar_tamano" => numero(valor, clave, 64.0, 220.0)?,
            "bloqueo_reloj_grosor" => {
                let v = entero(valor, clave, 100, 900)?;
                if v.parse::<u16>().is_ok_and(|n| n % 100 != 0) {
                    return Err("«bloqueo_reloj_grosor» va de 100 en 100".into());
                }
                v
            }
            "salvapantallas_estilo" => {
                let v = texto(valor, clave)?;
                if bookos_shell::EstiloSalvapantallas::desde_nombre(&v).is_none() {
                    return Err("«salvapantallas_estilo» debe ser estela o rebote".into());
                }
                v
            }
            "bloqueo_reloj_fuente" => {
                let v = texto(valor, clave)?;
                if bookos_shell::FuenteReloj::desde_nombre(&v).is_none() {
                    return Err("«bloqueo_reloj_fuente» no es un estilo de reloj conocido".into());
                }
                v
            }
            "bloqueo_animaciones"
            | "bloqueo_fecha"
            | "bloqueo_nombre"
            | "bloqueo_energia"
            | "bloqueo_medios"
            | "actividades"
            | "actividades_animaciones"
            | "temporizador_siempre_visible"
            | "sonido_volumen"
            | "sonidos_interfaz"
            | "genio"
            | "dock_esquiva"
            | "panel_esquiva"
            | "toque_para_clic"
            | "scroll_natural"
            | "pantalla_tactil" => booleano(valor, clave)?,
            "tema" => {
                let v = texto(valor, clave)?;
                if !matches!(v.as_str(), "claro" | "oscuro" | "automatico") {
                    return Err("«tema» debe ser claro, oscuro o automatico".into());
                }
                v
            }
            "efectos" => {
                let v = texto(valor, clave)?;
                if !matches!(v.as_str(), "completos" | "reducidos") {
                    return Err("«efectos» debe ser completos o reducidos".into());
                }
                v
            }
            "alto_contraste"
            | "bloqueo_huella"
            | "brillo_automatico_pantalla"
            | "brillo_automatico_teclado" => booleano(valor, clave)?,
            "dock_tamano" => entero(valor, clave, 32, 80)?,
            "teclado_retardo" => entero(valor, clave, 150, 1500)?,
            "teclado_velocidad" => entero(valor, clave, 0, 60)?,
            "volumen_paso" | "volumen_paso_mantener" | "brillo_paso" | "brillo_paso_mantener" => {
                let maximo = bookos_shell::Pasos::maximo_de(clave)
                    .expect("las cuatro claves de este brazo son pasos");
                entero(
                    valor,
                    clave,
                    bookos_shell::Pasos::MINIMO.into(),
                    maximo.into(),
                )?
            }
            "acento" => {
                let v = texto(valor, clave)?;
                if bookos_shell::tema::Acento::desde_nombre(&v).is_none() {
                    return Err("«acento» no pertenece a la paleta de BookOS".into());
                }
                v
            }
            "luz_nocturna" => {
                let v = texto(valor, clave)?;
                if !matches!(v.as_str(), "no" | "si" | "programada") {
                    return Err("«luz_nocturna» debe ser no, si o programada".into());
                }
                v
            }
            "luz_nocturna_temperatura" => entero(
                valor,
                clave,
                bookos_shell::LuzNocturna::MINIMA as u64,
                bookos_shell::LuzNocturna::MAXIMA as u64,
            )?,
            "tema_claro_desde" | "tema_oscuro_desde" | "luz_nocturna_desde"
            | "luz_nocturna_hasta" => {
                let v = texto(valor, clave)?;
                let valida = v
                    .split_once(':')
                    .and_then(|(h, m)| h.parse::<u8>().ok().zip(m.parse::<u8>().ok()))
                    .is_some_and(|(h, m)| h < 24 && m < 60);
                if !valida {
                    return Err(format!("«{clave}» debe tener formato HH:MM"));
                }
                v
            }
            // Derivada y de solo lectura. Se ignora en vez de rechazarse para
            // que un cliente pueda devolver tal cual el objeto que leyó de
            // `GetConfig` sin que la llamada entera falle.
            "tema_efectivo" | "luz_nocturna_activa" | "bloqueo_reloj_fuentes" => continue,
            otro => return Err(format!("clave de configuración desconocida: «{otro}»")),
        };
        salida.push((clave.clone(), guardado));
    }
    Ok(salida)
}

pub enum Aviso {
    Recargar(String),
    NoMolestar(Option<bool>, mpsc::Sender<bool>),
    /// Configuración de pantallas y por dónde devolver el veredicto.
    Salidas(Vec<Peticion>, mpsc::Sender<Result<(), String>>),
    /// Algo cambió en el hardware: volver a censar y avisar por la señal.
    Redetectar,
    /// La actividad y el nombre único de la conexión que la publicó.
    Actividad(bookos_shell::actividad::Estado, Option<String>),
    CerrarActividad(String),
    /// Una conexión del bus se ha ido: la app se cerró, se colgó o la mataron.
    RemitenteCaido(String),
    AbrirActividadPrevisualizacion(String),
    CapturarTecla(bool),
    PrevisualizarSalvapantallas(
        bookos_shell::EstiloSalvapantallas,
        mpsc::Sender<Result<(), String>>,
    ),
}

#[derive(Deserialize)]
struct ActividadJson {
    #[serde(default)]
    activo: bool,
    #[serde(default)]
    pausado: bool,
    #[serde(default)]
    titulo: String,
    #[serde(default)]
    subtitulo: String,
    #[serde(default)]
    posicion_ms: i64,
    #[serde(default)]
    duracion_ms: i64,
    #[serde(default)]
    restante_ms: i64,
    #[serde(default = "volumen_defecto")]
    volumen: u8,
    #[serde(default)]
    nivel: f32,
    // Aleatorio y repetición: la isla pinta encendidos sus dos botones. Van con
    // `default` como todo lo demás, así que una app que no los mande sigue
    // publicando igual que antes.
    #[serde(default)]
    aleatorio: bool,
    #[serde(default)]
    repetir: bool,
    #[serde(default)]
    portada: String,
    #[serde(default)]
    cola: Vec<ItemColaJson>,
    #[serde(default)]
    metricas: Vec<MetricaJson>,
}

#[derive(Deserialize)]
struct MetricaJson {
    #[serde(default)]
    etiqueta: String,
    #[serde(default)]
    valor: f32,
    #[serde(default)]
    texto: String,
    #[serde(default)]
    critica: bool,
}

#[derive(Deserialize)]
struct ItemColaJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    titulo: String,
    #[serde(default)]
    artista: String,
    #[serde(default)]
    duracion_ms: i64,
    #[serde(default)]
    favorita: bool,
    #[serde(default)]
    actual: bool,
    /// Carátula como data: URL. El reproductor solo la manda cuando cambia la
    /// cola; vacía significa «la de antes».
    #[serde(default)]
    portada: String,
}

fn volumen_defecto() -> u8 {
    100
}

struct Servidor {
    canal: Sender<Aviso>,
    compartido: Arc<Compartido>,
}

impl Servidor {
    fn do_not_disturb(&self, value: Option<bool>) -> zbus::fdo::Result<bool> {
        let (tx, rx) = mpsc::channel();
        self.canal
            .send(Aviso::NoMolestar(value, tx))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        rx.recv_timeout(ESPERA)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
}

#[zbus::interface(name = "org.bookos.Desktop")]
impl Servidor {
    fn get_do_not_disturb(&self) -> zbus::fdo::Result<bool> {
        self.do_not_disturb(None)
    }
    fn set_do_not_disturb(&self, enabled: bool) -> zbus::fdo::Result<bool> {
        self.do_not_disturb(Some(enabled))
    }

    /// Solicita recargar una sección. Devuelve `true` cuando la orden pudo
    /// entregarse al bucle del compositor.
    fn reload_config(&self, section: String) -> bool {
        self.canal.send(Aviso::Recargar(section)).is_ok()
    }

    /// Qué sabe hacer este backend.
    fn get_capabilities(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
        self.compartido.capacidades()
    }

    fn get_screensaver_preview(&self, estilo: &str) -> zbus::fdo::Result<String> {
        let estilo = bookos_shell::EstiloSalvapantallas::desde_nombre(estilo).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs("el estilo debe ser estela o rebote".into())
        })?;
        let reducidos = bookos_shell::Config::cargar().efectos == bookos_shell::Efectos::Reducidos;
        crate::salvapantallas::miniatura(estilo, reducidos)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    fn preview_screensaver(&self, estilo: &str) -> (bool, String) {
        let Some(estilo) = bookos_shell::EstiloSalvapantallas::desde_nombre(estilo) else {
            return (false, "el estilo debe ser estela o rebote".into());
        };
        let (tx, rx) = mpsc::channel();
        if self
            .canal
            .send(Aviso::PrevisualizarSalvapantallas(estilo, tx))
            .is_err()
        {
            return (
                false,
                "el compositor no pudo recibir la vista previa".into(),
            );
        }
        match rx.recv_timeout(ESPERA) {
            Ok(Ok(())) => (true, String::new()),
            Ok(Err(err)) => (false, err),
            Err(err) => (
                false,
                format!("el compositor no respondió a la vista previa: {err}"),
            ),
        }
    }

    /// El censo de salidas conectadas.
    fn get_outputs(&self) -> Vec<Salida> {
        self.compartido.salidas()
    }

    /// Instantánea completa para Settings. JSON mantiene el contrato legible
    /// para clientes que no usan Rust y permite añadir campos sin romper ABI.
    fn get_config(&self) -> String {
        config_json(&bookos_shell::Config::cargar()).to_string()
    }

    /// Valida y persiste una actualización parcial. Ninguna clave se escribe si
    /// una sola es inválida, de modo que Settings puede mostrar el error sin
    /// dejar el escritorio a medio configurar.
    fn apply_config(&self, json: String) -> (bool, String) {
        let valores = match validar_config_json(&json) {
            Ok(valores) => valores,
            Err(err) => return (false, err),
        };
        if let Err(err) = bookos_shell::guardar_configuracion(&valores) {
            return (false, format!("no se pudo guardar la configuración: {err}"));
        }
        if self.canal.send(Aviso::Recargar("all".into())).is_err() {
            return (
                false,
                "se guardó, pero el compositor no pudo aplicarla en vivo".into(),
            );
        }
        (true, String::new())
    }

    fn get_key_remaps(&self) -> String {
        crate::teclas::cargar().a_json()
    }

    fn apply_key_remaps(&self, json: String) -> (bool, String) {
        let remapeos = match crate::teclas::Remapeos::desde_json(&json) {
            Ok(remapeos) => remapeos,
            Err(err) => return (false, err),
        };
        if let Err(err) = crate::teclas::guardar(&remapeos) {
            return (false, format!("no se pudieron guardar los remapeos: {err}"));
        }
        if self.canal.send(Aviso::Recargar("teclas".into())).is_err() {
            return (
                false,
                "se guardaron, pero el compositor no pudo aplicarlos en vivo".into(),
            );
        }
        (true, String::new())
    }

    fn capture_key(&self, armar: bool) -> bool {
        self.canal.send(Aviso::CapturarTecla(armar)).is_ok()
    }

    /// Aplica una configuración de pantallas. `(ok, error)`: `ok=false` nunca
    /// va con `error` vacío, para que la interfaz no pueda enseñar un éxito que
    /// no ocurrió.
    fn apply_output_config(&self, config: Vec<Peticion>) -> (bool, String) {
        let (respuesta, espera) = mpsc::channel();
        if self.canal.send(Aviso::Salidas(config, respuesta)).is_err() {
            return (false, "el compositor no está escuchando".into());
        }
        match espera.recv_timeout(ESPERA) {
            Ok(Ok(())) => (true, String::new()),
            Ok(Err(err)) => (false, err),
            Err(_) => (
                false,
                "el compositor no contestó a tiempo; no se ha cambiado nada".into(),
            ),
        }
    }

    /// Publica una tarea viva. El identificador está en una lista cerrada: una
    /// app cualquiera no puede convertir la isla en una segunda bandeja de
    /// notificaciones.
    fn publish_activity(
        &self,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        app_id: String,
        kind: String,
        state_json: String,
    ) -> bool {
        let Some(clase) = clase_permitida(&app_id, &kind) else {
            tracing::warn!(app_id, kind, "publicador de actividad rechazado");
            return false;
        };
        if state_json.len() > 8 * 1024 * 1024 {
            tracing::warn!(app_id, "estado de actividad demasiado grande");
            return false;
        }
        let Ok(json) = serde_json::from_str::<ActividadJson>(&state_json) else {
            tracing::warn!(app_id, "estado de actividad no válido");
            return false;
        };
        let portada = decodificar_portada(&json.portada);
        let estado = bookos_shell::actividad::Estado {
            app_id,
            clase,
            activo: json.activo,
            pausado: json.pausado,
            titulo: limitar(json.titulo, 120),
            subtitulo: limitar(json.subtitulo, 160),
            posicion_ms: json.posicion_ms,
            duracion_ms: json.duracion_ms.max(0),
            restante_ms: json.restante_ms,
            volumen: json.volumen.min(100),
            nivel: json.nivel.clamp(0.0, 1.0),
            aleatorio: json.aleatorio,
            repetir: json.repetir,
            portada,
            cola: json
                .cola
                .into_iter()
                .take(50)
                .map(|i| bookos_shell::actividad::ItemCola {
                    id: limitar(i.id, 512),
                    titulo: limitar(i.titulo, 120),
                    artista: limitar(i.artista, 120),
                    duracion_ms: i.duracion_ms.max(0),
                    favorita: i.favorita,
                    actual: i.actual,
                    portada: decodificar_portada(&i.portada),
                })
                .collect(),
            // Cinco filas caben en la tarjeta; más es un monitor que se ha
            // convertido en una tabla.
            metricas: json
                .metricas
                .into_iter()
                .take(6)
                .map(|m| bookos_shell::actividad::Metrica {
                    etiqueta: limitar(m.etiqueta, 24),
                    valor: if m.valor.is_finite() {
                        m.valor.clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                    texto: limitar(m.texto, 16),
                    critica: m.critica,
                })
                .collect(),
        };
        let remitente = cabecera.sender().map(|u| u.to_string());
        self.canal.send(Aviso::Actividad(estado, remitente)).is_ok()
    }

    fn close_activity(&self, app_id: String) -> bool {
        if clase_permitida(&app_id, "").is_none() {
            return false;
        }
        self.canal.send(Aviso::CerrarActividad(app_id)).is_ok()
    }

    /// Abre una actividad ya publicada para inspeccionarla durante el
    /// desarrollo. En release se rechaza: una aplicación no debe poder forzar
    /// que su tarjeta se despliegue sobre lo que esté haciendo el usuario.
    fn preview_activity_open(&self, app_id: String) -> bool {
        if !cfg!(debug_assertions) || clase_permitida(&app_id, "").is_none() {
            return false;
        }
        self.canal
            .send(Aviso::AbrirActividadPrevisualizacion(app_id))
            .is_ok()
    }

    #[zbus(signal)]
    async fn outputs_changed(emisor: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    /// Orden del usuario hacia la app que publicó la actividad.
    #[zbus(signal)]
    async fn activity_action(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        app_id: &str,
        action: &str,
        value: &str,
    ) -> zbus::Result<()>;
}

pub fn recibir(state: &mut crate::state::BookosComp, aviso: Aviso) {
    match aviso {
        Aviso::NoMolestar(value, reply) => {
            let enabled = state
                .shell
                .as_mut()
                .map(|s| s.no_molestar(value))
                .unwrap_or(false);
            let _ = reply.send(enabled);
            state.needs_redraw = true;
        }
        Aviso::Recargar(seccion) => recargar(state, &seccion),
        Aviso::PrevisualizarSalvapantallas(estilo, respuesta) => {
            let _ = respuesta.send(crate::salvapantallas::previsualizar(state, estilo));
        }
        Aviso::Salidas(peticion, respuesta) => {
            let r = crate::pantallas::aplicar(state, peticion);
            if let Err(err) = r.as_ref() {
                tracing::warn!("configuración de pantallas rechazada: {err}");
            }
            // Que el otro extremo se haya rendido (timeout) no es motivo para
            // deshacer lo aplicado: el cambio ya está en la pantalla.
            let _ = respuesta.send(r);
        }
        Aviso::Redetectar => redetectar(state),
        Aviso::CapturarTecla(armar) => state.captura_tecla = armar,
        Aviso::Actividad(estado, remitente) => {
            match remitente {
                Some(r) => state.remitentes_actividad.insert(estado.app_id.clone(), r),
                None => state.remitentes_actividad.remove(&estado.app_id),
            };
            if let Some(shell) = state.shell.as_mut() {
                shell.publicar_actividad(estado);
                state.needs_redraw = true;
            }
        }
        Aviso::CerrarActividad(app_id) => {
            state.remitentes_actividad.remove(&app_id);
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_actividad(&app_id))
            {
                state.needs_redraw = true;
            }
        }
        Aviso::RemitenteCaido(nombre) => {
            let huerfanas: Vec<String> = state
                .remitentes_actividad
                .iter()
                .filter(|(_, r)| **r == nombre)
                .map(|(app_id, _)| app_id.clone())
                .collect();
            for app_id in huerfanas {
                state.remitentes_actividad.remove(&app_id);
                tracing::info!(app_id, "actividad retirada: su app ya no está en el bus");
                if state
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.cerrar_actividad(&app_id))
                {
                    state.needs_redraw = true;
                }
            }
        }
        Aviso::AbrirActividadPrevisualizacion(app_id) => {
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.abrir_actividad_previsualizacion(&app_id))
            {
                state.needs_redraw = true;
            }
        }
    }
}

fn clase_permitida(app_id: &str, kind: &str) -> Option<bookos_shell::actividad::Clase> {
    use bookos_shell::actividad::Clase;
    match (app_id, kind) {
        ("com.bookos.player", "player" | "") => Some(Clase::Player),
        ("com.bookos.clock", "timer" | "") => Some(Clase::Timer),
        ("com.bookos.voicerecorder", "recorder" | "") => Some(Clase::Recorder),
        // Book Share vive en Ajustes: es quien recibe y envía por Quick Share.
        ("com.bookos.settings", "transfer" | "") => Some(Clase::Transfer),
        // Las copias y movimientos largos entre carpetas y discos.
        ("com.bookos.explorer", "transfer" | "") => Some(Clase::Transfer),
        ("com.bookos.calendar", "calendar" | "") => Some(Clase::Calendar),
        ("com.bookos.systemmonitor", "monitor" | "") => Some(Clase::Monitor),
        // bookos-budsd, el servicio de sesión de los Galaxy Buds.
        ("com.bookos.buds", "auriculares" | "") => Some(Clase::Auriculares),
        _ => None,
    }
}

fn limitar(mut texto: String, max: usize) -> String {
    if texto.chars().count() > max {
        texto = texto.chars().take(max).collect();
    }
    texto
}

fn decodificar_portada(valor: &str) -> Option<bookos_shell::actividad::Portada> {
    let b64 = valor.strip_prefix("data:image/")?.split_once(',')?.1;
    if b64.len() > 6 * 1024 * 1024 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let (rgba, width, height) = bookos_shell::decodificar_imagen(&bytes)?;
    Some(bookos_shell::actividad::Portada {
        rgba,
        width,
        height,
    })
}

fn recargar(state: &mut crate::state::BookosComp, seccion: &str) {
    const CONOCIDAS: &[&str] = &[
        "lockscreen",
        "bloqueo",
        "activities",
        "actividades",
        "wallpaper",
        "fondo",
        "appearance",
        "apariencia",
        "keys",
        "teclas",
        "desktop",
        "escritorio",
        "brightness",
        "brillo",
        "entrada",
        "teclado",
        "channel",
        "canal",
        "all",
    ];
    if !CONOCIDAS.contains(&seccion) {
        tracing::warn!(seccion, "sección de ajustes desconocida");
        return;
    }
    if matches!(seccion, "keys" | "teclas" | "all") {
        state.teclas = crate::teclas::cargar();
    }
    if matches!(seccion, "channel" | "canal" | "all")
        && let Some(shell) = state.shell.as_mut()
    {
        shell.aplicar_canal(bookos_shell::marca::Canal::leer());
    }
    let mut config = bookos_shell::Config::cargar();

    // Velocidad, toque y scroll caían en los dispositivos solo al aparecer.
    if matches!(seccion, "entrada" | "all") {
        state.entrada = std::mem::take(&mut config.entrada);
        if let Some(aplicar) = state.aplicar_entrada.as_ref() {
            aplicar(&state.entrada, state.touchpad_activo);
        }
        // Sin táctil no hay con qué usar el teclado en pantalla.
        if !state.entrada.pantalla_tactil {
            crate::teclado::retirar(state);
        }
    }
    let repeticion = (config.teclado_retardo, config.teclado_velocidad);
    if matches!(seccion, "teclado" | "all")
        && repeticion != state.teclado_repeticion
        && let Some(teclado) = state.seat.get_keyboard()
    {
        // Lo reciben también los clientes ya conectados: son ellos los que
        // repiten, y sin el aviso seguirían con el ritmo de cuando entraron.
        teclado.change_repeat_info(repeticion.1 as i32, repeticion.0 as i32);
        state.teclado_repeticion = repeticion;
    }
    if matches!(seccion, "teclado" | "all") && config.teclado != state.teclado_activo {
        if let Some(teclado) = state.seat.get_keyboard() {
            let xkb = crate::state::distribucion_teclado(config.teclado.as_deref());
            match teclado.set_xkb_config(state, xkb) {
                Ok(()) => state.teclado_activo = config.teclado.clone(),
                Err(err) => tracing::warn!(?err, "no se pudo aplicar la distribución de teclado"),
            }
        }
    }

    if matches!(seccion, "desktop" | "escritorio" | "all")
        && state
            .shell
            .as_mut()
            .is_some_and(|shell| shell.dock_tamano(config.dock_tamano))
    {
        state.recolocar_encajadas();
        state.revisar_barras();
    }

    // El tema se aplica **antes** de tocar el fondo: `Eleccion::para_el_tema`
    // pregunta por el que esté puesto, y con el orden al revés se recargaría la
    // imagen del tema viejo. Lo aplica el shell, que es quien tiene que
    // repintarse con él, y ese cambio es global al proceso.
    if matches!(seccion, "appearance" | "apariencia" | "all") {
        state.modo_tema = config.modo_tema;
        state.horas_tema = (config.tema_claro_desde, config.tema_oscuro_desde);
        bookos_shell::tema::aplicar_modo(config.modo_tema);
        bookos_shell::tema::aplicar_alto_contraste(config.alto_contraste);
        // `config.tema` ya viene resuelto contra el reloj por `Config::cargar`.
        if let Some(shell) = state.shell.as_mut() {
            shell.aplicar_apariencia(config.tema, config.acento);
        }
        crate::portal::apariencia_cambiada(state.bus_portal.as_ref());
        // Y el despertar del próximo cambio, que pudo cambiar de hora o dejar
        // de existir si el modo pasó a fijo.
        crate::apariencia::programar_cambio(state);
    }
    if matches!(
        seccion,
        "wallpaper" | "fondo" | "appearance" | "apariencia" | "all"
    ) {
        state.fondo_config = crate::fondo::Eleccion {
            ambos: config.fondo.clone(),
            claro: config.fondo_claro.clone(),
            oscuro: config.fondo_oscuro.clone(),
        };
        // Y qué familia queda marcada en la tarjeta de Apariencia.
        bookos_shell::fondos::poner_elegida(
            state
                .fondo_config
                .para_el_tema()
                .map(std::path::Path::new)
                .and_then(bookos_shell::fondos::familia_de),
        );
        // Aquí no se compara con lo que había: quien pide recargar el fondo lo
        // pide porque cambió el fichero, y ahorrarse la comparación es más
        // barato que llevar la cuenta de qué imagen estaba puesta.
        state.recargar_fondo = true;
    }

    if let Some(shell) = state.shell.as_mut() {
        if matches!(seccion, "lockscreen" | "bloqueo" | "all") {
            shell.aplicar_bloqueo_config(config.bloqueo);
        }
        if matches!(seccion, "activities" | "actividades" | "all") {
            shell.aplicar_actividades_config(config.actividades);
        }
        if matches!(seccion, "appearance" | "apariencia" | "all") {
            shell.aplicar_efectos_config(config.efectos);
        }
    }
    if matches!(seccion, "appearance" | "apariencia" | "all") {
        crate::backend::programar_fondo_animado(state);
    }
    // Solo si cambió: `recargar` descarta lo encendido a mano, y cualquier
    // otro ajuste guardado desde Settings pasa por aquí con «all».
    if matches!(seccion, "brightness" | "brillo" | "all")
        && config.luz_nocturna != state.luz_nocturna.config
    {
        crate::luz_nocturna::recargar(state, config.luz_nocturna);
    }
    // «desktop» y «escritorio» estaban en `CONOCIDAS` sin que nada los
    // atendiera: el número de escritorios guardado desde Ajustes no llegaba a
    // la vista de Meta+W hasta volver a iniciar sesión.
    // Lo mismo que hace Meta+Alt+B, pero solo si lo guardado no coincide con
    // lo que hay: alternar sin mirar invertiría la barra en cada recarga.
    if matches!(seccion, "desktop" | "escritorio" | "all") {
        let mut cambio = false;
        for (cual, esquiva) in [
            (crate::shell::Barra::Panel, config.panel_esquiva),
            (crate::shell::Barra::Dock, config.dock_esquiva),
        ] {
            if let Some(shell) = state.shell.as_mut()
                && (shell.visibilidad(cual) == crate::shell::Visibilidad::Esquivando) != esquiva
            {
                shell.alternar_visibilidad(cual);
                cambio = true;
            }
        }
        if cambio {
            state.recolocar_encajadas();
            state.revisar_barras();
        }
    }
    if matches!(seccion, "desktop" | "escritorio" | "all")
        && crate::escritorios::ajustar(state, config.escritorios, &config.nombres_escritorios)
    {
        let activo = crate::escritorios::activo_aqui(state);
        let nombres = state.escritorios.nombres().to_vec();
        if let Some(shell) = state.shell.as_mut() {
            shell.actualizar_vista_escritorios(activo, nombres);
        }
    }
    if seccion == "all" {
        state.sonido_volumen = config.sonido_volumen;
        state.sonidos_interfaz = config.sonidos_interfaz;
        state.pasos = config.pasos;
        state.genio_activo = config.genio;
    }
    if matches!(seccion, "brightness" | "brillo" | "all") {
        crate::brillo_auto::elegir(state, config.brillo_automatico);
    }
    if matches!(seccion, "lockscreen" | "bloqueo" | "all") {
        state.bloqueo_inactividad = std::time::Duration::from_secs(config.bloqueo_inactividad);
        state.salvapantallas.inactividad =
            std::time::Duration::from_secs(config.salvapantallas_inactividad);
        state.salvapantallas.apagado = std::time::Duration::from_secs(config.apagado_inactividad);
        state.salvapantallas.estilo = config.salvapantallas_estilo;
        state.atenuacion.inactividad =
            std::time::Duration::from_secs(config.atenuacion_inactividad);
        state.suspension_inactividad =
            std::time::Duration::from_secs(config.suspension_inactividad);
        crate::backend::programar_bloqueo_inactividad(state);
        crate::backend::programar_suspension_inactividad(state);
    }
    crate::publicar::publicar(&state.fondo_config);
    state.needs_redraw = true;
}

/// Reconcilia conectores y configuración sin reiniciar el compositor. Un EDID
/// conocido recupera su perfil; uno nuevo se extiende a la derecha. Al quitar
/// una pantalla desaparece del mapa y la primera restante pasa a principal si
/// hacía falta.
fn redetectar(state: &mut crate::state::BookosComp) {
    let Some(censar) = state.censar_pantallas.take() else {
        return;
    };
    let salidas = censar();
    state.censar_pantallas = Some(censar);
    if salidas == state.pantallas.compartido.salidas() {
        return;
    }
    state.pantallas.compartido.publicar(salidas.clone());
    let guardadas = crate::pantallas::cargar();
    let mut peticion = crate::pantallas::peticion_de(&salidas);
    let mut derecha = peticion
        .iter()
        .filter(|p| p.activa)
        .map(|p| {
            let (w, _) =
                crate::pantallas::tamano_logico(p.ancho, p.alto, p.escala, &p.transformacion);
            p.x + w
        })
        .max()
        .unwrap_or(0);

    for (p, salida) in peticion.iter_mut().zip(&salidas) {
        if p.activa {
            continue;
        }
        if let Some(g) = guardadas.iter().find(|g| g.id == p.id) {
            if !g.activa {
                continue;
            }
            let modo_existe = salida.modos.iter().any(|m| {
                m.ancho == g.ancho && m.alto == g.alto && m.refresco_mhz == g.refresco_mhz
            });
            if modo_existe {
                *p = g.clone();
                continue;
            }
        }
        let Some(modo) = salida
            .modos
            .iter()
            .find(|m| m.preferido)
            .or_else(|| salida.modos.first())
        else {
            continue;
        };
        p.activa = true;
        p.ancho = modo.ancho;
        p.alto = modo.alto;
        p.refresco_mhz = modo.refresco_mhz;
        p.x = derecha;
        p.y = 0;
        let (w, _) = crate::pantallas::tamano_logico(p.ancho, p.alto, p.escala, &p.transformacion);
        derecha += w;
    }
    if !peticion.iter().any(|p| p.activa)
        && let Some((p, salida)) = peticion.first_mut().zip(salidas.first())
        && let Some(modo) = salida
            .modos
            .iter()
            .find(|m| m.preferido)
            .or_else(|| salida.modos.first())
    {
        p.activa = true;
        p.principal = true;
        p.ancho = modo.ancho;
        p.alto = modo.alto;
        p.refresco_mhz = modo.refresco_mhz;
        p.x = 0;
        p.y = 0;
    }
    crate::pantallas::normalizar(&mut peticion);
    if crate::pantallas::validar(&salidas, &peticion).is_err() {
        // Un perfil antiguo puede solaparse con una pantalla desconocida. En
        // hotplug prima conservar imagen: se crea una fila válida y Settings
        // puede recolocarla después en cualquier dirección.
        let principal = peticion
            .iter()
            .position(|p| p.activa && p.principal)
            .or_else(|| peticion.iter().position(|p| p.activa))
            .unwrap_or(0);
        peticion[principal].principal = true;
        let mut x = 0;
        let orden =
            std::iter::once(principal).chain((0..peticion.len()).filter(|i| *i != principal));
        for i in orden {
            if !peticion[i].activa {
                continue;
            }
            peticion[i].principal = i == principal;
            peticion[i].x = x;
            peticion[i].y = 0;
            let (w, _) = crate::pantallas::tamano_logico(
                peticion[i].ancho,
                peticion[i].alto,
                peticion[i].escala,
                &peticion[i].transformacion,
            );
            x += w;
        }
        crate::pantallas::normalizar(&mut peticion);
    }
    // La vuelta atrás de un hotplug debe representar solo conectores que aún
    // existen; una salida desenchufada no puede formar parte del rollback.
    state.pantallas.ultima = crate::pantallas::peticion_de(&salidas);
    if let Err(err) = crate::pantallas::aplicar(state, peticion) {
        tracing::warn!("no se pudo aplicar el cambio de monitores en caliente: {err}");
        avisar_salidas(state);
    }
}

/// Emite `OutputsChanged`. Se llama desde el hilo del compositor con la
/// conexión bloqueante que ya se guarda en el estado: emitir una señal es
/// escribir en el socket del bus, no hay que despertar a zbus para eso.
pub fn avisar_salidas(state: &crate::state::BookosComp) {
    let Some(conexion) = state.bus_ajustes.as_ref() else {
        return;
    };
    let r = conexion.emit_signal(None::<&str>, RUTA, NOMBRE, "OutputsChanged", &());
    if let Err(err) = r {
        tracing::warn!("no se pudo emitir OutputsChanged: {err}");
    }
}

pub fn accion_actividad(
    state: &crate::state::BookosComp,
    accion: &bookos_shell::actividad::Accion,
) {
    let Some(conexion) = state.bus_ajustes.as_ref() else {
        return;
    };
    if let Err(err) = conexion.emit_signal(
        None::<&str>,
        RUTA,
        NOMBRE,
        "ActivityAction",
        &(&accion.app_id, &accion.nombre, &accion.valor),
    ) {
        tracing::warn!("no se pudo enviar la acción de actividad: {err}");
    }
}

pub fn avisar_tecla_capturada(state: &crate::state::BookosComp, evdev: u32, copilot: bool) {
    let Some(conexion) = state.bus_ajustes.as_ref() else {
        return;
    };
    if let Err(err) =
        conexion.emit_signal(None::<&str>, RUTA, NOMBRE, "KeyCaptured", &(evdev, copilot))
    {
        tracing::warn!("no se pudo emitir KeyCaptured: {err}");
    }
}

pub fn arrancar(
    canal: Sender<Aviso>,
    compartido: Arc<Compartido>,
) -> Option<zbus::blocking::Connection> {
    let canal_vigia = canal.clone();
    let conexion = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(NOMBRE))
        .and_then(|b| {
            b.serve_at(
                RUTA,
                Servidor {
                    canal: canal.clone(),
                    compartido,
                },
            )
        })
        .and_then(|b| b.build());
    match conexion {
        Ok(conexion) => {
            tracing::info!("interfaz de ajustes de BookOS en el bus");
            vigilar_remitentes(&conexion, canal_vigia);
            Some(conexion)
        }
        Err(err) => {
            tracing::warn!("sin interfaz de ajustes de BookOS: {err}");
            None
        }
    }
}

/// Avisa cuando se va del bus una conexión cualquiera, para retirar las
/// actividades que publicó.
///
/// Sin esto la isla dependía de que la app llamase a `CloseActivity` antes de
/// irse, y ninguna lo hace al cerrar la ventana: el Player y el Reloj dejaban
/// su actividad colgada hasta reiniciar la sesión. Tampoco sirve mirar las
/// ventanas: el Player sigue sonando con la ventana cerrada en la bandeja, y
/// una app colgada o matada con `kill` no avisa de nada. La conexión al bus sí
/// muere con el proceso, pase lo que pase.
///
/// La regla filtra en el propio bus por `new_owner` vacío: solo llegan las
/// desapariciones, no cada nombre que se pide o se cede.
fn vigilar_remitentes(conexion: &zbus::blocking::Connection, canal: Sender<Aviso>) {
    let conexion = conexion.clone();
    let hilo = std::thread::Builder::new()
        .name("vigia-actividades".into())
        .spawn(move || {
            let señales = zbus::blocking::fdo::DBusProxy::new(&conexion)
                .and_then(|p| p.receive_name_owner_changed_with_args(&[(2, "")]));
            let señales = match señales {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!("sin vigía de actividades: {err}");
                    return;
                }
            };
            for señal in señales {
                let Ok(args) = señal.args() else { continue };
                // Solo los nombres únicos (`:1.42`): son los que se anotan al
                // publicar, y un nombre conocido que cambia de dueño no
                // significa que el proceso haya muerto.
                if let zbus::names::BusName::Unique(nombre) = args.name()
                    && canal
                        .send(Aviso::RemitenteCaido(nombre.to_string()))
                        .is_err()
                {
                    return;
                }
            }
        });
    if let Err(err) = hilo {
        tracing::warn!("sin vigía de actividades: {err}");
    }
}

/// Vuelve a pedir los nombres del bus si alguien los suelta.
///
/// El log de sesión mostró `monitor_name_lost` para los tres nombres del
/// escritorio en el mismo milisegundo, sin que el compositor hiciera nada: la
/// conexión seguía viva pero sin nombre, y como solo se piden al arrancar
/// se quedaron así hasta reiniciar sesión (sin notificaciones ni ajustes).
/// Pedir un nombre que ya es nuestro devuelve Ok, así que basta con repetir.
/// Cada 5 s: un sondeo cuesta una llamada al bus y perder el nombre unos
/// segundos es aceptable.
pub fn vigilar_nombres(conexiones: Vec<(zbus::blocking::Connection, &'static str)>) {
    if conexiones.is_empty() {
        return;
    }
    let hilo = std::thread::Builder::new()
        .name("vigia-bus".into())
        .spawn(move || {
            let mut caidos: Vec<&'static str> = Vec::new();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                for (conexion, nombre) in &conexiones {
                    match conexion.request_name(*nombre) {
                        Ok(()) => {
                            if let Some(i) = caidos.iter().position(|n| n == nombre) {
                                caidos.swap_remove(i);
                                tracing::info!("nombre {nombre} recuperado en el bus");
                            }
                        }
                        // Otro escritorio lo tiene (Plasma debajo, desarrollando
                        // anidado): se avisa una vez y se sigue intentando.
                        Err(err) if !caidos.contains(nombre) => {
                            caidos.push(nombre);
                            tracing::warn!("no se pudo recuperar {nombre}: {err}");
                        }
                        Err(_) => {}
                    }
                }
            }
        });
    if let Err(err) = hilo {
        tracing::warn!("sin vigía de nombres del bus: {err}");
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::pantallas::{Modo, Salida};

    fn salida_de_prueba() -> Salida {
        Salida {
            id: "SDC-ATNA40YK-0".into(),
            conector: "eDP-1".into(),
            fabricante: "SDC".into(),
            modelo: "ATNA40YK".into(),
            serie: String::new(),
            mm_ancho: 300,
            mm_alto: 190,
            activa: true,
            modos: vec![Modo {
                ancho: 2880,
                alto: 1800,
                refresco_mhz: 120_000,
                preferido: true,
                actual: true,
            }],
            escala: 1.75,
            escalas: crate::pantallas::ESCALAS.to_vec(),
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

    #[test]
    fn la_api_de_configuracion_valida_antes_de_escribir() {
        let valores = validar_config_json(
            r#"{"version":1,"tema":"automatico","cursor":32,"toque_para_clic":true}"#,
        )
        .expect("actualización válida");
        assert!(valores.contains(&("tema".into(), "automatico".into())));
        assert!(valores.contains(&("cursor".into(), "32".into())));
        assert!(valores.contains(&("toque_para_clic".into(), "si".into())));

        assert!(validar_config_json(r#"{"cursor":12.5}"#).is_err());
        assert!(
            validar_config_json(
                r#"{"dock":[{"exec":"a:b","etiqueta":"A","icono":"a","app_id":"a"}]}"#
            )
            .is_err()
        );
        assert!(validar_config_json(r#"{"inventada":true}"#).is_err());
        for json in [
            r#"{"dock_tamano":31}"#,
            r#"{"dock_tamano":81}"#,
            r#"{"dock_tamano":50.5}"#,
            r#"{"bloqueo_huella":"si"}"#,
        ] {
            assert!(validar_config_json(json).is_err(), "{json}");
        }
        assert!(validar_config_json(r#"{"dock_tamano":80,"bloqueo_huella":true}"#).is_ok());
    }

    /// Al pulsar hasta 10 y al mantener hasta 30, en volumen y en brillo.
    #[test]
    fn los_pasos_tienen_un_techo_distinto_al_pulsar_y_al_mantener() {
        for ok in [
            r#"{"volumen_paso":10}"#,
            r#"{"brillo_paso":1}"#,
            r#"{"volumen_paso_mantener":30}"#,
            r#"{"brillo_paso_mantener":30}"#,
        ] {
            assert!(validar_config_json(ok).is_ok(), "{ok}");
        }
        for mal in [
            r#"{"volumen_paso":11}"#,
            r#"{"brillo_paso":0}"#,
            r#"{"volumen_paso_mantener":31}"#,
            r#"{"brillo_paso_mantener":0}"#,
        ] {
            assert!(validar_config_json(mal).is_err(), "{mal}");
        }
    }

    /// La luz nocturna con los mismos límites que lee `panel.conf`: fuera de
    /// ellos el compositor rechaza la actualización entera.
    #[test]
    fn el_estilo_del_reloj_se_valida() {
        let valores =
            validar_config_json(r#"{"bloqueo_reloj_fuente":"clasica","bloqueo_reloj_grosor":700}"#)
                .expect("estilo válido");
        assert!(valores.contains(&("bloqueo_reloj_fuente".into(), "clasica".into())));
        assert!(valores.contains(&("bloqueo_reloj_grosor".into(), "700".into())));
        for json in [
            r#"{"bloqueo_reloj_fuente":"Comic Sans"}"#,
            r#"{"bloqueo_reloj_grosor":450}"#,
            r#"{"bloqueo_reloj_grosor":1000}"#,
        ] {
            assert!(validar_config_json(json).is_err(), "{json}");
        }
        // La lista de instaladas se puede devolver tal cual: se ignora.
        assert!(validar_config_json(r#"{"bloqueo_reloj_fuentes":[]}"#).is_ok());
    }

    #[test]
    fn la_luz_nocturna_se_valida() {
        for json in [
            r#"{"luz_nocturna":true}"#,
            r#"{"luz_nocturna":"siempre"}"#,
            r#"{"luz_nocturna_temperatura":2499}"#,
            r#"{"luz_nocturna_temperatura":8001}"#,
            r#"{"luz_nocturna_desde":"24:00"}"#,
        ] {
            assert!(validar_config_json(json).is_err(), "{json}");
        }
        let ok = validar_config_json(
            r#"{"luz_nocturna":"programada","luz_nocturna_temperatura":3400,
                "luz_nocturna_desde":"21:30","luz_nocturna_hasta":"07:00",
                "luz_nocturna_activa":true}"#,
        )
        .expect("configuración válida");
        assert_eq!(ok.len(), 4, "`luz_nocturna_activa` es de solo lectura");
    }

    #[test]
    fn get_config_entrega_un_contrato_versionado_completo() {
        let json = config_json(&bookos_shell::Config::default());
        assert_eq!(json["version"], 1);
        assert!(json["dock"].is_array());
        assert!(json.get("bloqueo_inactividad").is_some());
        assert_eq!(json["suspension_inactividad"], 0);
        assert_eq!(json["salvapantallas_inactividad"], 1200);
        assert_eq!(json["apagado_inactividad"], 1800);
        assert_eq!(
            validar_config_json(r#"{"salvapantallas_inactividad":0}"#),
            Ok(vec![("salvapantallas_inactividad".into(), "0".into())])
        );
        assert!(validar_config_json(r#"{"salvapantallas_inactividad":86401}"#).is_err());
        assert!(json.get("velocidad_touchpad").is_some());
        assert_eq!(json["pantalla_tactil"], true);
        assert_eq!(
            validar_config_json(r#"{"pantalla_tactil":false}"#),
            Ok(vec![("pantalla_tactil".into(), "no".into())])
        );
        assert_eq!(json["sonido_volumen"], true);
        assert_eq!(
            validar_config_json(r#"{"sonido_volumen":false}"#),
            Ok(vec![("sonido_volumen".into(), "no".into())])
        );
        assert_eq!(json["sonidos_interfaz"], true);
        assert_eq!(
            validar_config_json(r#"{"sonidos_interfaz":false}"#),
            Ok(vec![("sonidos_interfaz".into(), "no".into())])
        );
    }

    /// `tema` es la preferencia y `tema_efectivo` el color pintado. Con
    /// «automatico» solo el segundo dice de qué color va la interfaz, que es
    /// justo el caso donde un cliente se quedaba sin respuesta.
    #[test]
    fn el_tema_efectivo_es_el_color_pintado_y_no_la_preferencia() {
        use bookos_shell::tema::{self, ModoTema, Tema};

        let antes = tema::actual();
        let config = bookos_shell::Config {
            modo_tema: ModoTema::Automatico,
            ..Default::default()
        };
        // El tema es global y los tests que crean un `ShellHost` lo reescriben
        // en paralelo. La primera llamada a `config_json` carga la base de
        // fuentes (`bloqueo_reloj_fuentes`) y tarda: se hace aquí, antes de
        // fijar el tema, para no abrir esa ventana entre fijarlo y leerlo.
        config_json(&config);

        tema::aplicar(Tema::Claro);
        let json = config_json(&config);
        assert_eq!(json["tema"], "automatico");
        assert_eq!(json["tema_efectivo"], "claro");

        tema::aplicar(Tema::Oscuro);
        assert_eq!(config_json(&config)["tema_efectivo"], "oscuro");

        // Y devolver lo leído no puede reventar `ApplyConfig`.
        let valores = validar_config_json(&config_json(&config).to_string())
            .expect("el objeto que entrega GetConfig tiene que poder reenviarse");
        assert!(
            !valores.iter().any(|(k, _)| k == "tema_efectivo"),
            "«tema_efectivo» es derivado: no se guarda"
        );

        tema::aplicar(antes);
    }

    /// El censo y las capacidades tienen que poder ir y volver **por el bus de
    /// verdad**: las firmas las genera zvariant desde los tipos, y un campo que
    /// no se pueda serializar no se ve compilando, se ve al llamar.
    ///
    /// El servidor se publica sin pedir `org.bookos.Desktop` y se le habla por
    /// el nombre único de la conexión: así el test no le quita el nombre a una
    /// sesión de BookOS que esté corriendo en la misma máquina.
    #[test]
    fn el_censo_viaja_por_el_bus() {
        let (emisor, receptor) = smithay::reexports::calloop::channel::channel::<Aviso>();
        // Sin bucle de eventos nadie atiende el canal. Se suelta el extremo de
        // lectura para que el envío falle en el acto en vez de esperar los
        // cinco segundos de `ESPERA`: lo que se prueba aquí es la firma y el
        // camino del error, no el reloj.
        drop(receptor);
        let compartido = Arc::new(Compartido::default());
        compartido.publicar(vec![salida_de_prueba()]);

        let servidor = Servidor {
            canal: emisor,
            compartido: compartido.clone(),
        };
        let conexion = match zbus::blocking::connection::Builder::session()
            .and_then(|b| b.serve_at(RUTA, servidor))
            .and_then(|b| b.build())
        {
            Ok(c) => c,
            // Sin bus de sesión —una compilación en un contenedor, por
            // ejemplo— no hay nada que probar aquí.
            Err(_) => return,
        };
        let yo = conexion
            .unique_name()
            .expect("la conexión tiene nombre único")
            .to_string();

        let cliente = zbus::blocking::Connection::session().expect("bus de sesión");
        let proxy = zbus::blocking::Proxy::new(&cliente, yo.as_str(), RUTA, NOMBRE).expect("proxy");

        let salidas: Vec<Salida> = proxy.call("GetOutputs", &()).expect("GetOutputs");
        assert_eq!(salidas, vec![salida_de_prueba()]);

        let caps: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
            proxy.call("GetCapabilities", &()).expect("GetCapabilities");
        assert_eq!(
            u32::try_from(&caps["version"]).unwrap(),
            crate::pantallas::VERSION_CONTRATO
        );
        assert!(caps.contains_key("escalas"));
        assert!(caps.contains_key("fractional_scale"));

        let config: String = proxy.call("GetConfig", &()).expect("GetConfig");
        let config: serde_json::Value = serde_json::from_str(&config).expect("JSON de config");
        assert_eq!(config["version"], 1);

        // Y una configuración imposible tiene que volver como fallo con motivo,
        // no como un éxito silencioso. Nadie atiende el canal en el test, así
        // que lo que se comprueba es que la llamada contesta y que `ok` es
        // falso con un `error` no vacío.
        let (ok, error): (bool, String) = proxy
            .call(
                "ApplyOutputConfig",
                &(Vec::<crate::pantallas::Peticion>::new(),),
            )
            .expect("ApplyOutputConfig");
        assert!(!ok);
        assert!(!error.is_empty());
    }
}
