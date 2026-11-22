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
//! **Quién puede llamar.** Leer está abierto. Escribir no: `ApplyConfig` y
//! `ApplyOutputConfig` solo los acepta de Ajustes, `ApplyKeyRemaps` y
//! `CaptureKey` de la app de teclas y de Ajustes, y `PublishActivity` /
//! `CloseActivity` solo del programa que corresponde a ese `app_id`. El
//! programa se saca del PID que da el bus; ver [`programa_que_llama`].
//!
//! GetAppearance() -> s
//!     El aspecto resuelto en JSON: tema, acento, alto contraste, efectos
//!     reducidos y los papeles de la paleta dinámica del tema que se pinta.
//!     Ver [`aspecto_json`].
//!
//! signal AppearanceChanged(s aspecto)
//!     Cambió cualquiera de esas cosas. Lleva el mismo JSON.
//!
//! ReloadConfig(s seccion) -> b
//!     Relee `panel.conf`. "palette"/"paleta" no relee nada: Ajustes acaba
//!     de escribir `palette.json` y se avisa con `AppearanceChanged`.
//!     Secciones: "lockscreen"/"bloqueo",
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
//! Lock() -> b / Suspend() -> b
//!     Lo mismo que el menú de energía del escritorio: suspender echa antes
//!     el bloqueo y apaga la pantalla, así que al despertar sale el candado.
//!     Un `systemctl suspend` desde fuera no lo hacía: el compositor no
//!     atiende el bloqueo de logind. Solo Ajustes y Zenith.
//!
//! SetAppearance(s json) -> (b ok, s error)
//!     Lo mismo que `ApplyConfig`, pero solo con las claves de `APARIENCIA`
//!     (tema, alto contraste, efectos, luz nocturna) y abierto también a
//!     Zenith, para «pon el modo oscuro» por voz. El resto de la
//!     configuración sigue siendo solo de Ajustes.
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
//!     nadie y sale por `KeyCaptured`, solo hacia quien armó. No es la respuesta del método porque
//!     esperar a una persona dentro de él ocupa el ejecutor de zbus y para
//!     el resto del bus. Con `false` se desarma: quien se canse de esperar
//!     tiene que desarmar, o la tecla se tragaría más tarde.
//!
//! signal OutputsChanged()
//!     Algo cambió en las pantallas —se aplicó una configuración, se enchufó o
//!     se quitó un monitor—. Quien la reciba vuelve a pedir `GetOutputs`.
//!
//! signal KeyCaptured(u evdev, b copilot)
//!     La tecla capturada, con destino: solo la recibe quien armó la
//!     captura. `copilot` si era el acorde Meta+Mayús+F23.
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

// 0.1.0
// modified 0.1.1 -- expone salvapantallas, atenuación, apagado y pantalla táctil a Ajustes
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
    json["fondo_encaje"] = config.fondo_encaje.nombre().into();
    json["apagado_inactividad"] = config.apagado_inactividad.into();
    // Fuera del `json!`: con una clave más, el macro pasa del límite de
    // recursión de rustc.
    json["sonido_volumen"] = config.sonido_volumen.into();
    json["sonidos_interfaz"] = config.sonidos_interfaz.into();
    json["genio"] = config.genio.into();
    json["panel_tamano"] = config.panel_tamano.nombre().into();
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
    json["teclado_pantalla"] = config.entrada.teclado_pantalla.into();
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
    // Solo lectura, como `tema_efectivo`: los widgets que el panel sabe
    // construir, en el orden en que Ajustes los ofrece. Fuera de `json!`
    // porque la macro ya está en su límite de recursión.
    json["widgets_disponibles"] = bookos_shell::WIDGETS_DISPONIBLES.into();
    json["captura_cursor"] = config.captura_cursor.into();
    // Fuera del `json!` de arriba: con una clave más pasa el límite de recursión del macro.
    json["texto_tamano"] = config.texto_tamano.into();
    json["teclas_especiales"] = config.teclas_especiales.into();
    json["teclas_lentas"] = config.teclas_lentas.into();
    json["bateria_porcentaje"] = config.bateria_porcentaje.into();
    json["bateria_tiempo"] = config.bateria_tiempo.into();
    json["bateria_colores"] = config.bateria_colores.into();
    json["captura_retardo"] = config.captura_retardo.into();
    json["apertura_ventanas"] = config.apertura_ventanas.nombre().into();
    json["vrr_refresco_max"] = config.vrr_refresco_max.into();
    json["notificaciones_silenciadas"] = config.notificaciones_silenciadas.clone().into();
    json["bajo_consumo"] = config.bajo_consumo.into();
    json["agitar_cursor"] = config.agitar_cursor.into();
    json["modo_concentracion"] = config.modo_concentracion.clone().into();
    json["concentracion_auto_juego"] = config.concentracion_auto_juego.into();
    json["concentracion_permitidas"] = serde_json::json!(config.concentracion_permitidas);
    json
}

// 0.1.0
// modified 0.1.1 -- valida las claves nuevas de salvapantallas, atenuación, apagado y táctil
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

    // Un widget que no existe no rompe el panel —se ignora al construirlo—,
    // pero tampoco avisa: quien lo escribe se queda sin él sin saber por qué.
    let widget_conocido = |s: String, clave: &str| {
        if bookos_shell::WIDGETS_DISPONIBLES.contains(&s.as_str()) {
            Ok(s)
        } else {
            Err(format!("«{clave}» lleva un widget desconocido: «{s}»"))
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
                    widget_conocido(seguro(&texto(valor, clave)?, clave, &[','])?, clave)?
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
                let nombres = items
                    .iter()
                    .map(|v| texto(v, clave).and_then(|s| seguro(&s, clave, &[','])))
                    .collect::<Result<Vec<_>, _>>()?;
                if clave == "derecha" {
                    for (n, nombre) in nombres.iter().enumerate() {
                        widget_conocido(nombre.clone(), clave)?;
                        // El panel busca el widget por nombre para el clic: con
                        // dos iguales, uno de ellos no respondería.
                        if nombres[..n].contains(nombre) {
                            return Err(format!("«derecha» repite el widget «{nombre}»"));
                        }
                    }
                    let en_centro =
                        objeto.get("centro").and_then(|v| v.as_str()) == Some("privacidad");
                    if !en_centro && !nombres.iter().any(|n| n == "privacidad") {
                        return Err(
                            "«derecha» debe incluir «privacidad»: los puntos de privacidad no se pueden quitar"
                                .into(),
                        );
                    }
                }
                nombres.join(", ")
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
            "texto_tamano" => entero(valor, clave, 100, 200)?,
            "teclas_lentas" => entero(valor, clave, 0, 2000)?,
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
            "fondo_encaje" => {
                let v = texto(valor, clave)?;
                if bookos_shell::EncajeFondo::desde_nombre(&v).is_none() {
                    return Err(
                        "«fondo_encaje» debe ser rellenar, ajustar, ancho, estirar o centrar"
                            .into(),
                    );
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
            | "pantalla_tactil"
            | "teclado_pantalla" => booleano(valor, clave)?,
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
            | "bateria_porcentaje"
            | "bateria_tiempo"
            | "bateria_colores"
            | "bloqueo_huella"
            | "brillo_automatico_pantalla"
            | "brillo_automatico_teclado"
            | "teclas_especiales"
            | "captura_cursor"
            | "bajo_consumo"
            | "agitar_cursor"
            | "concentracion_auto_juego" => booleano(valor, clave)?,
            "modo_concentracion" => {
                let v = texto(valor, clave)?.trim().to_lowercase();
                if v.contains('\n') || v.contains('=') {
                    return Err("«modo_concentracion» contiene caracteres no válidos".into());
                }
                v
            }
            "captura_retardo" => {
                let n = valor
                    .as_u64()
                    .filter(|n| {
                        bookos_shell::RETARDOS_CAPTURA
                            .iter()
                            .any(|r| u64::from(*r) == *n)
                    })
                    .ok_or("«captura_retardo» debe ser 0, 3, 5 o 10")?;
                n.to_string()
            }
            "dock_tamano" => entero(valor, clave, 32, 80)?,
            "panel_tamano" => {
                let v = texto(valor, clave)?;
                if bookos_shell::TamanoPanel::desde_nombre(&v).is_none() {
                    return Err("«panel_tamano» debe ser pequeno, mediano o grande".into());
                }
                v
            }
            "apertura_ventanas" => {
                let v = texto(valor, clave)?;
                if bookos_shell::AperturaVentanas::desde_nombre(&v).is_none() {
                    return Err(
                        "«apertura_ventanas» debe ser normal, maximizada o pantalla_completa".into(),
                    );
                }
                v
            }
            "vrr_refresco_max" => {
                let n = valor
                    .as_u64()
                    .filter(|n| *n == 0 || (24..=360).contains(n))
                    .ok_or("«vrr_refresco_max» debe ser 0 (sin límite) o entre 24 y 360 Hz")?;
                n.to_string()
            }
            "notificaciones_silenciadas" => {
                let lista = valor
                    .as_array()
                    .ok_or("«notificaciones_silenciadas» debe ser una lista de aplicaciones")?;
                let mut limpias = Vec::with_capacity(lista.len());
                for item in lista {
                    let s = item
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && !s.contains(',') && !s.contains('\n'))
                        .ok_or(
                            "cada entrada de «notificaciones_silenciadas» debe ser texto sin comas",
                        )?;
                    if !limpias.iter().any(|x: &String| x.eq_ignore_ascii_case(s)) {
                        limpias.push(s.to_string());
                    }
                }
                limpias.join(", ")
            }
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
            "concentracion_permitidas" => {
                let mapa = valor.as_object().ok_or(
                    "«concentracion_permitidas» debe ser un objeto { modo: [apps...] }",
                )?;
                for (modo, apps_val) in mapa {
                    let m = modo.trim().to_lowercase();
                    if m.is_empty()
                        || !m
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        return Err(format!("modo de concentración no válido: «{modo}»"));
                    }
                    let lista = apps_val.as_array().ok_or_else(|| {
                        format!("las apps de «{modo}» deben ser una lista de cadenas")
                    })?;
                    let mut limpias = Vec::with_capacity(lista.len());
                    for item in lista {
                        let s = item
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty() && !s.contains(',') && !s.contains('\n'))
                            .ok_or(
                                "cada app de concentración debe ser texto no vacío sin comas",
                            )?;
                        if !limpias.iter().any(|x: &String| x.eq_ignore_ascii_case(s)) {
                            limpias.push(s.to_string());
                        }
                    }
                    salida.push((format!("concentracion_{m}"), limpias.join(", ")));
                }
                continue;
            }
            // Derivada y de solo lectura. Se ignora en vez de rechazarse para
            // que un cliente pueda devolver tal cual el objeto que leyó de
            // `GetConfig` sin que la llamada entera falle.
            "tema_efectivo"
            | "luz_nocturna_activa"
            | "bloqueo_reloj_fuentes"
            | "widgets_disponibles" => {
                continue;
            }
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
    /// La actividad que cerrar y quién lo pide.
    CerrarActividad(String, Option<String>),
    /// Una conexión del bus se ha ido: la app se cerró, se colgó o la mataron.
    RemitenteCaido(String),
    /// `org.freedesktop.ScreenSaver.Inhibit`: la cookie y quién la pidió.
    // añadido en la 0.2.2
    Inhibir(u32, String),
    /// `UnInhibit` de esa cookie.
    // añadido en la 0.2.2
    Desinhibir(u32),
    AbrirActividadPrevisualizacion(String),
    /// Armar o desarmar, y el nombre único de quien lo pide.
    // modified 0.2.2
    CapturarTecla(bool, String),
    PrevisualizarSalvapantallas(
        bookos_shell::EstiloSalvapantallas,
        mpsc::Sender<Result<(), String>>,
    ),
    /// Bloquear o suspender desde Zenith, por el mismo camino que el menú.
    // añadido en la 0.2.3
    Energia(bookos_shell::confirmacion::Energia),
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
    // añadido en la 0.2.2 -- modo del control de ruido; solo los auriculares.
    #[serde(default)]
    anc: Option<u8>,
    #[serde(default)]
    portada: String,
    // añadido en la 0.2.0 -- la fase del asistente; el resto de clases no la mandan.
    #[serde(default)]
    fase: String,
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

// 0.1.0
fn volumen_defecto() -> u8 {
    100
}

struct Servidor {
    canal: Sender<Aviso>,
    compartido: Arc<Compartido>,
}

impl Servidor {
    // 0.1.0
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
    // 0.1.0
    fn get_do_not_disturb(&self) -> zbus::fdo::Result<bool> {
        self.do_not_disturb(None)
    }
    // 0.1.0
    fn set_do_not_disturb(&self, enabled: bool) -> zbus::fdo::Result<bool> {
        self.do_not_disturb(Some(enabled))
    }

    /// Solicita recargar una sección. Devuelve `true` cuando la orden pudo
    /// entregarse al bucle del compositor.
    // 0.1.0
    fn reload_config(&self, section: String) -> bool {
        self.canal.send(Aviso::Recargar(section)).is_ok()
    }

    /// Qué sabe hacer este backend.
    // 0.1.0
    fn get_capabilities(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
        self.compartido.capacidades()
    }

    // 0.1.1
    fn get_screensaver_preview(&self, estilo: &str) -> zbus::fdo::Result<String> {
        let estilo = bookos_shell::EstiloSalvapantallas::desde_nombre(estilo).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs("el estilo debe ser estela o rebote".into())
        })?;
        let reducidos = bookos_shell::Config::cargar().efectos == bookos_shell::Efectos::Reducidos;
        crate::salvapantallas::miniatura(estilo, reducidos)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    // 0.1.1
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
    /// El aspecto ya resuelto: tema, acento, contraste, movimiento y los
    /// papeles de la paleta del tema que se pinta. Ver [`aspecto_json`].
    // añadido en la 0.2.2
    fn get_appearance(&self) -> String {
        aspecto_json()
    }

    // 0.1.0
    fn get_outputs(&self) -> Vec<Salida> {
        self.compartido.salidas()
    }

    /// Instantánea completa para Settings. JSON mantiene el contrato legible
    /// para clientes que no usan Rust y permite añadir campos sin romper ABI.
    // 0.1.0
    fn get_config(&self) -> String {
        config_json(&bookos_shell::Config::cargar()).to_string()
    }

    /// Valida y persiste una actualización parcial. Ninguna clave se escribe si
    /// una sola es inválida, de modo que Settings puede mostrar el error sin
    /// dejar el escritorio a medio configurar.
    // modified 0.2.2 -- solo Ajustes
    async fn apply_config(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        json: String,
    ) -> (bool, String) {
        if !autorizado(conexion, &cabecera, "ApplyConfig", CONFIGURAN).await {
            return (false, "solo Ajustes puede cambiar la configuración".into());
        }
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

    // añadido en la 0.2.3
    async fn set_appearance(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        json: String,
    ) -> (bool, String) {
        if !autorizado(conexion, &cabecera, "SetAppearance", APARIENCIA_PERMITIDOS).await {
            return (
                false,
                "solo Ajustes y Zenith pueden cambiar la apariencia".into(),
            );
        }
        let valores = match validar_config_json(&json) {
            Ok(valores) => valores,
            Err(err) => return (false, err),
        };
        if let Some((clave, _)) = valores
            .iter()
            .find(|(c, _)| !APARIENCIA.contains(&c.as_str()))
        {
            return (false, format!("«{clave}» no es una clave de apariencia"));
        }
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

    // añadido en la 0.2.3
    async fn lock(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
    ) -> bool {
        use bookos_shell::confirmacion::Energia;
        autorizado(conexion, &cabecera, "Lock", APARIENCIA_PERMITIDOS).await
            && self.canal.send(Aviso::Energia(Energia::Bloquear)).is_ok()
    }

    // añadido en la 0.2.3
    async fn suspend(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
    ) -> bool {
        use bookos_shell::confirmacion::Energia;
        autorizado(conexion, &cabecera, "Suspend", APARIENCIA_PERMITIDOS).await
            && self.canal.send(Aviso::Energia(Energia::Suspender)).is_ok()
    }

    // 0.1.0
    fn get_key_remaps(&self) -> String {
        crate::teclas::cargar().a_json()
    }

    // modified 0.2.2 -- solo la app de teclas y Ajustes
    async fn apply_key_remaps(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        json: String,
    ) -> (bool, String) {
        if !autorizado(conexion, &cabecera, "ApplyKeyRemaps", TECLEAN).await {
            return (
                false,
                "solo la app de teclas puede cambiar los remapeos".into(),
            );
        }
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

    /// La tecla sale solo hacia quien armó la captura (`KeyCaptured` con
    /// destino), y solo él puede desarmarla. Antes era una señal de difusión:
    /// cualquier proceso que escuchara el bus veía la tecla, y cualquiera podía
    /// armar la captura en bucle y leer lo que se escribía.
    // modified 0.2.2 -- solo la app de teclas, y la tecla va solo a ella
    async fn capture_key(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        armar: bool,
    ) -> bool {
        let Some(remitente) = cabecera.sender().map(|u| u.to_string()) else {
            return false;
        };
        if !autorizado(conexion, &cabecera, "CaptureKey", TECLEAN).await {
            return false;
        }
        self.canal
            .send(Aviso::CapturarTecla(armar, remitente))
            .is_ok()
    }

    /// Aplica una configuración de pantallas. `(ok, error)`: `ok=false` nunca
    /// va con `error` vacío, para que la interfaz no pueda enseñar un éxito que
    /// no ocurrió.
    // modified 0.2.2 -- solo Ajustes
    async fn apply_output_config(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        config: Vec<Peticion>,
    ) -> (bool, String) {
        if !autorizado(conexion, &cabecera, "ApplyOutputConfig", CONFIGURAN).await {
            return (false, "solo Ajustes puede cambiar las pantallas".into());
        }
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
    ///
    /// Un rechazo es un error de D-Bus con el motivo, no un `false`: con el
    /// `false` la app no sabía si fallaba el JSON, el permiso o el tipo, y el
    /// motivo solo quedaba en el registro del compositor. Las apps que ya
    /// existían ignoran el error o lo pasan a `Err` (revisadas el 2026-11-16).
    // modified 0.2.2 -- el app_id tiene que corresponder al programa que llama
    // modified 0.2.2 -- los rechazos devuelven el motivo
    async fn publish_activity(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        app_id: String,
        kind: String,
        state_json: String,
    ) -> zbus::fdo::Result<bool> {
        // Sin capturar `app_id`: se mueve al estado al final.
        fn rechazo(motivo: String) -> zbus::fdo::Error {
            tracing::warn!(motivo, "actividad rechazada");
            zbus::fdo::Error::Failed(motivo)
        }
        let Some(clase) = clase_permitida(&app_id, &kind) else {
            return Err(rechazo(if clase_permitida(&app_id, "").is_some() {
                format!("«{app_id}» no puede publicar actividades de tipo «{kind}»")
            } else {
                format!("«{app_id}» no está en la lista de apps que pueden usar la isla")
            }));
        };
        if !autorizado(
            conexion,
            &cabecera,
            "PublishActivity",
            programas_de(&app_id),
        )
        .await
        {
            return Err(rechazo(format!(
                "el programa que llama no es el de «{app_id}»"
            )));
        }
        if state_json.len() > 8 * 1024 * 1024 {
            return Err(rechazo(format!("«{app_id}»: el estado pasa de 8 MB")));
        }
        let json = serde_json::from_str::<ActividadJson>(&state_json)
            .map_err(|e| rechazo(format!("«{app_id}»: estado no válido: {e}")))?;
        let clase = match clase {
            bookos_shell::actividad::Clase::Asistente(_) => {
                let Some(fase) = bookos_shell::actividad::Fase::desde(&json.fase) else {
                    return Err(rechazo(format!(
                        "fase del asistente desconocida: «{}»",
                        json.fase
                    )));
                };
                bookos_shell::actividad::Clase::Asistente(fase)
            }
            otra => otra,
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
            anc: json.anc.filter(|m| *m <= 3),
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
        self.canal
            .send(Aviso::Actividad(estado, remitente))
            .map(|_| true)
            .map_err(|_| rechazo("el compositor no está escuchando".into()))
    }

    // modified 0.2.2 -- el app_id tiene que corresponder al programa que llama
    // modified 0.2.2 -- los rechazos devuelven el motivo
    async fn close_activity(
        &self,
        #[zbus(connection)] conexion: &zbus::Connection,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        app_id: String,
    ) -> zbus::fdo::Result<bool> {
        if clase_permitida(&app_id, "").is_none() {
            return Err(zbus::fdo::Error::Failed(format!(
                "«{app_id}» no está en la lista de apps que pueden usar la isla"
            )));
        }
        if !autorizado(conexion, &cabecera, "CloseActivity", programas_de(&app_id)).await {
            return Err(zbus::fdo::Error::Failed(format!(
                "el programa que llama no es el de «{app_id}»"
            )));
        }
        let remitente = cabecera.sender().map(|u| u.to_string());
        self.canal
            .send(Aviso::CerrarActividad(app_id, remitente))
            .map(|_| true)
            .map_err(|_| zbus::fdo::Error::Failed("el compositor no está escuchando".into()))
    }

    /// Abre una actividad ya publicada para inspeccionarla durante el
    /// desarrollo. En release se rechaza: una aplicación no debe poder forzar
    /// que su tarjeta se despliegue sobre lo que esté haciendo el usuario.
    // 0.1.0
    fn preview_activity_open(&self, app_id: String) -> bool {
        if !cfg!(debug_assertions) || clase_permitida(&app_id, "").is_none() {
            return false;
        }
        self.canal
            .send(Aviso::AbrirActividadPrevisualizacion(app_id))
            .is_ok()
    }

    #[zbus(signal)]
    // 0.1.0
    async fn outputs_changed(emisor: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    /// El aspecto cambió; lleva el mismo JSON que `GetAppearance`.
    #[zbus(signal)]
    // añadido en la 0.2.2
    async fn appearance_changed(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        aspecto: &str,
    ) -> zbus::Result<()>;

    /// Orden del usuario hacia la app que publicó la actividad.
    #[zbus(signal)]
    // 0.1.0
    async fn activity_action(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        app_id: &str,
        action: &str,
        value: &str,
    ) -> zbus::Result<()>;
}

// 0.1.0
// modified 0.1.1 -- atiende la previsualización del salvapantallas desde Ajustes
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
        Aviso::Energia(energia) => {
            crate::keybinds::hacer(state, bookos_shell::Accion::EnergiaConfirmada(energia))
        }
        Aviso::CapturarTecla(true, remitente) => state.captura_tecla = Some(remitente),
        // Solo desarma quien armó: si no, otra app podría quitarle la captura
        // a la de teclas mientras espera.
        Aviso::CapturarTecla(false, remitente) => {
            if state.captura_tecla.as_ref() == Some(&remitente) {
                state.captura_tecla = None;
            }
        }
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
        Aviso::CerrarActividad(app_id, remitente) => {
            // Solo cierra quien la publicó. Con cuatro BookOS Player abiertos
            // (12-11), los tres sin cola mandaban CloseActivity cada 5 s y
            // borraban la isla del que sí sonaba.
            if let (Some(dueño), Some(r)) =
                (state.remitentes_actividad.get(&app_id), remitente.as_ref())
                && dueño != r
            {
                tracing::debug!(app_id, remitente = r, "cierre de actividad ajena ignorado");
                return;
            }
            state.remitentes_actividad.remove(&app_id);
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_actividad(&app_id))
            {
                state.needs_redraw = true;
            }
        }
        Aviso::Inhibir(cookie, remitente) => {
            state.inhibiciones_bus.insert(cookie, remitente);
            crate::handlers::inhibicion_cambiada(state);
        }
        Aviso::Desinhibir(cookie) => {
            if state.inhibiciones_bus.remove(&cookie).is_some() {
                crate::handlers::inhibicion_cambiada(state);
            }
        }
        Aviso::RemitenteCaido(nombre) => {
            // Quien armó la captura se fue sin desarmarla: la siguiente tecla
            // se habría tragado sin que nadie la recibiera.
            if state.captura_tecla.as_ref() == Some(&nombre) {
                state.captura_tecla = None;
            }
            let antes = state.inhibiciones_bus.len();
            state.inhibiciones_bus.retain(|_, r| *r != nombre);
            if state.inhibiciones_bus.len() != antes {
                tracing::info!(nombre, "inhibición soltada: su app ya no está en el bus");
                crate::handlers::inhibicion_cambiada(state);
            }
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

// 0.1.0
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
        // Zenith; la fase llega en el estado y la pone `publish_activity`.
        ("com.bookos.assistant", "asistente" | "") => {
            Some(Clase::Asistente(bookos_shell::actividad::Fase::Escuchando))
        }
        _ => None,
    }
}

/// Los programas que pueden publicar con cada `app_id`.
///
/// Sin esto la lista cerrada de `clase_permitida` solo cerraba el texto: un
/// proceso cualquiera podía decir que era `com.bookos.player` y pintar en la
/// isla con su aspecto. Hay dos nombres donde la versión GPUI se instala con
/// otro binario mientras se desarrolla (`cargo run` deja el del paquete).
// añadido en la 0.2.2
fn programas_de(app_id: &str) -> &'static [&'static str] {
    match app_id {
        "com.bookos.player" => &["bookos-player"],
        "com.bookos.clock" => &["bookos-clock"],
        "com.bookos.voicerecorder" => &["bookos-voicerecorder"],
        // Book Share publica como Ajustes, desde su propio binario.
        "com.bookos.settings" => &["bookos-settings", "bookos-bookshare"],
        "com.bookos.explorer" => &["bookos-explorer"],
        "com.bookos.calendar" => &["bookos-calendar"],
        "com.bookos.systemmonitor" => &["bookos-system-monitor", "bookos-system-monitor-gpui"],
        "com.bookos.buds" => &["bookos-budsd"],
        "com.bookos.assistant" => &["bookos-assistant"],
        _ => &[],
    }
}

/// Quién puede cambiar la configuración del escritorio y las pantallas.
// añadido en la 0.2.2
const CONFIGURAN: &[&str] = &["bookos-settings"];
/// Las claves que se pueden cambiar sin ser Ajustes. Son las del aspecto y la
/// accesibilidad, que el usuario pide de viva voz («pon el modo oscuro»); nada
/// de pantallas, bloqueo, teclas ni privacidad.
// añadido en la 0.2.3
const APARIENCIA: &[&str] = &["tema", "alto_contraste", "efectos", "luz_nocturna"];
// añadido en la 0.2.3
const APARIENCIA_PERMITIDOS: &[&str] = &["bookos-settings", "bookos-assistant"];
/// Quién puede remapear teclas y capturar la siguiente pulsación.
// añadido en la 0.2.2
const TECLEAN: &[&str] = &["bookos-teclas", "bookos-settings"];

/// El programa que hay detrás de una llamada: el nombre del ejecutable, o el
/// del script si el ejecutable es Python (Zenith es `python3
/// /usr/bin/bookos-assistant`).
///
/// No es una frontera de seguridad entre procesos del mismo usuario: uno de
/// ellos puede lanzar ese mismo binario con `LD_PRELOAD` o escribir los
/// ficheros de configuración a mano. Lo que sí consigue es que una app que no
/// es del sistema no pueda usar la interfaz por accidente o a propósito sin
/// más, y que `CaptureKey` —que entrega pulsaciones, lo único que Wayland no
/// deja ver a otra app— no esté abierto a cualquiera.
///
/// El PID sale del bus (`GetConnectionUnixProcessID`), no del mensaje, que no
/// lo lleva. Tiene que ser `async`: un método síncrono esperando al bus deja
/// parada la conexión entera, igual que en el portal.
// añadido en la 0.2.2
async fn programa_que_llama(
    conexion: &zbus::Connection,
    cabecera: &zbus::message::Header<'_>,
) -> Option<String> {
    let remitente = cabecera.sender()?.to_owned();
    let pid = zbus::fdo::DBusProxy::new(conexion)
        .await
        .ok()?
        .get_connection_unix_process_id(remitente.into())
        .await
        .ok()?;
    programa_de_pid(pid)
}

// añadido en la 0.2.2
fn programa_de_pid(pid: u32) -> Option<String> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    // Tras actualizar el paquete el enlace apunta a «… (deleted)» hasta que
    // la app se reinicia; sigue siendo el mismo programa.
    let exe = exe.to_string_lossy();
    let nombre = nombre_base(exe.strip_suffix(" (deleted)").unwrap_or(&exe));
    if !nombre.starts_with("python") {
        return Some(nombre.to_string());
    }
    let linea = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let script = linea.split(|b| *b == 0).nth(1)?;
    Some(nombre_base(&String::from_utf8_lossy(script)).to_string())
}

// añadido en la 0.2.2
fn nombre_base(ruta: &str) -> &str {
    ruta.rsplit('/').next().unwrap_or(ruta)
}

/// Si quien llama está en la lista. Lo rechazado queda en el log con el
/// programa, para que una app nueva sepa por qué no le contestan.
// añadido en la 0.2.2
async fn autorizado(
    conexion: &zbus::Connection,
    cabecera: &zbus::message::Header<'_>,
    metodo: &str,
    permitidos: &[&str],
) -> bool {
    let programa = programa_que_llama(conexion, cabecera).await;
    let ok = programa.as_deref().is_some_and(|p| permitidos.contains(&p));
    if !ok {
        tracing::warn!(
            metodo,
            programa = programa.as_deref().unwrap_or("?"),
            "llamada rechazada"
        );
    }
    ok
}

// 0.1.0
fn limitar(mut texto: String, max: usize) -> String {
    if texto.chars().count() > max {
        texto = texto.chars().take(max).collect();
    }
    texto
}

// 0.1.0
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

// 0.1.0
// modified 0.1.1 -- aplica salvapantallas, atenuación y apagado propios; sin táctil retira el teclado en pantalla
fn recargar(state: &mut crate::state::BookosComp, seccion: &str) {
    // Ajustes acaba de escribir `palette.json`. No hay nada del compositor que
    // releer —`tema::roles_paleta` ya lee el fichero cada vez—, pero sí que
    // repintar y avisar a las apps, que antes no se enteraban hasta recuperar
    // el foco.
    if matches!(seccion, "palette" | "paleta") {
        state.needs_redraw = true;
        aspecto_cambiado(state.bus_ajustes.as_ref());
        return;
    }
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
        if !state.entrada.pantalla_tactil || !state.entrada.teclado_pantalla {
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

    // Los widgets no cambian el alto: se rehacen aparte, y el fotograma lo pide
    // el final de esta función.
    if matches!(seccion, "desktop" | "escritorio" | "all")
        && let Some(shell) = state.shell.as_mut()
    {
        bookos_shell::bateria_mostrar(
            config.bateria_porcentaje,
            config.bateria_tiempo,
            config.bateria_colores,
        );
        shell.widgets_panel(config.centro.as_deref(), &config.derecha);
    }
    // Con la misma lista de widgets `widgets_panel` no rehace nada: los que
    // dependen de ajustes globales (la pila de la batería) tienen que releerlos.
    if matches!(seccion, "desktop" | "escritorio" | "all") {
        crate::backend::refresh_panel(state);
    }

    // Un modificador pegado al apagar las teclas especiales se suelta aquí.
    // añadido en la 0.2.3
    if matches!(seccion, "entrada" | "all") {
        crate::teclas_accesibles::configurar(
            state,
            config.teclas_especiales,
            config.teclas_lentas,
        );
    }
    // El cursor y el texto de las aplicaciones. El cursor lo dibuja el
    // compositor para los clientes que piden una forma por nombre; los que
    // cargan el suyo (Qt, GTK) lo cogen de gsettings (`portal`) o al relanzarse.
    // añadido en la 0.2.3
    if matches!(seccion, "appearance" | "apariencia" | "all")
        && (config.cursor != state.cursor_nominal || config.texto_tamano != state.texto_tamano)
    {
        let cursor_cambia = config.cursor != state.cursor_nominal;
        state.cursor_nominal = config.cursor;
        state.texto_tamano = config.texto_tamano;
        if cursor_cambia {
            let escala = state
                .space
                .outputs()
                .map(|o| o.current_scale().fractional_scale())
                .max_by(f64::total_cmp)
                .unwrap_or(1.0);
            state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(
                escala,
                state.cursor_nominal,
            ));
        }
        // `Xft.dpi` y `Xcursor.size` de XWayland salen del mismo sitio.
        crate::xwayland::actualizar_escala(state);
        state.needs_redraw = true;
    }

    // `|` y no `||`: con el dock sin cambios el panel tiene que aplicarse igual.
    if matches!(seccion, "desktop" | "escritorio" | "all")
        && state.shell.as_mut().is_some_and(|shell| {
            shell.panel_tamano(config.panel_tamano) | shell.dock_tamano(config.dock_tamano)
        })
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
            // Ajustes avisa con esta misma sección al cambiar el estilo de iconos.
            shell.recargar_iconos();
        }
        // Antes de avisar al portal: este lee los efectos reducidos (para
        // `reduced-motion` y `enable-animations`) y con el orden al revés
        // contestaba con el valor de la vez anterior.
        // modificado en la 0.2.3
        if let Some(shell) = state.shell.as_mut() {
            shell.aplicar_efectos_config(config.efectos);
        }
        crate::portal::apariencia_cambiada(state);
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
            encaje: config.fondo_encaje,
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
        state.apertura_ventanas = config.apertura_ventanas;
        state.vrr_refresco_max = config.vrr_refresco_max;
        let cambio_ahorro = state.bajo_consumo != config.bajo_consumo;
        state.bajo_consumo = config.bajo_consumo;
        state.agitar_cursor.habilitado = config.agitar_cursor;
        state.modo_concentracion = config.modo_concentracion.clone();
        state.concentracion_auto_juego = config.concentracion_auto_juego;
        state.concentracion_permitidas = config.concentracion_permitidas.clone();
        if let Some(shell) = state.shell.as_mut() {
            shell.poner_apps_silenciadas(config.notificaciones_silenciadas.clone());
            shell.poner_bajo_consumo(config.bajo_consumo);
        }
        if cambio_ahorro {
            let perfil = if config.bajo_consumo {
                "power-saver"
            } else {
                "balanced"
            };
            crate::keybinds::hacer(state, bookos_shell::Accion::PerfilEnergia(perfil));
        }
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
// 0.1.0
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
// 0.1.0
pub fn avisar_salidas(state: &crate::state::BookosComp) {
    let Some(conexion) = state.bus_ajustes.as_ref() else {
        return;
    };
    let r = conexion.emit_signal(None::<&str>, RUTA, NOMBRE, "OutputsChanged", &());
    if let Err(err) = r {
        tracing::warn!("no se pudo emitir OutputsChanged: {err}");
    }
}

/// El aspecto que tiene que adoptar una app, resuelto por el escritorio para
/// que nadie tenga que leer `palette.json`, preguntar el tema por otro lado y
/// combinar las dos cosas con sus propios respaldos.
///
/// `paleta` son los papeles del tema que se está pintando, con las mismas
/// claves que `palette.json` (sin `--`). Es `null` sin color dinámico o con
/// alto contraste: la app usa entonces sus valores de respaldo. `version`
/// sube solo si una clave cambia de significado; añadir claves no la cambia.
// añadido en la 0.2.2
pub fn aspecto_json() -> String {
    use bookos_shell::tema;
    let claro = tema::es_claro();
    let acento = tema::acento();
    let canal = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let paleta = (!tema::alto_contraste())
        .then(|| {
            let ruta = std::path::PathBuf::from(std::env::var_os("HOME")?)
                .join(".config/bookos/palette.json");
            let json: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(ruta).ok()?).ok()?;
            json.get(if claro { "light" } else { "dark" }).cloned()
        })
        .flatten();
    serde_json::json!({
        "version": 1,
        "tema": if claro { "claro" } else { "oscuro" },
        "acento": format!("#{:02x}{:02x}{:02x}", canal(acento.r), canal(acento.g), canal(acento.b)),
        "alto_contraste": tema::alto_contraste(),
        "efectos_reducidos": tema::efectos_reducidos(),
        "paleta": paleta,
    })
    .to_string()
}

/// Emite `AppearanceChanged` a quien escuche. Es información pública, la misma
/// que da el portal: va en difusión.
// añadido en la 0.2.2
pub fn aspecto_cambiado(conexion: Option<&zbus::blocking::Connection>) {
    let Some(conexion) = conexion else {
        return;
    };
    if let Err(err) = conexion.emit_signal(
        None::<&str>,
        RUTA,
        NOMBRE,
        "AppearanceChanged",
        &(aspecto_json(),),
    ) {
        tracing::warn!("no se pudo emitir AppearanceChanged: {err}");
    }
}

// 0.1.0
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

// modified 0.2.2 -- con destino: solo lo recibe quien armó la captura
pub fn avisar_tecla_capturada(
    state: &crate::state::BookosComp,
    destino: &str,
    evdev: u32,
    copilot: bool,
) {
    let Some(conexion) = state.bus_ajustes.as_ref() else {
        return;
    };
    if let Err(err) = conexion.emit_signal(
        Some(destino),
        RUTA,
        NOMBRE,
        "KeyCaptured",
        &(evdev, copilot),
    ) {
        tracing::warn!("no se pudo emitir KeyCaptured: {err}");
    }
}

// 0.1.0
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
            crate::inhibir::arrancar(&conexion, canal.clone());
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
// 0.1.0
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
// 0.1.0
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

    /// Un widget con una errata se quedaría fuera del panel sin que nadie se
    /// enterase; repetido, uno de los dos no recibiría los clics.
    #[test]
    fn los_widgets_del_panel_se_validan_por_nombre() {
        let valores = validar_config_json(
            r#"{"centro":"reloj","derecha":["monitor","trafico","privacidad","consumo","red"]}"#,
        )
        .expect("widgets conocidos");
        assert!(valores.contains(&(
            "derecha".into(),
            "monitor, trafico, privacidad, consumo, red".into()
        )));
        assert!(valores.contains(&("centro".into(), "reloj".into())));
        // Los puntos de privacidad se pueden mover a cualquier sitio (incluso al centro),
        // pero no se pueden quitar.
        assert!(validar_config_json(r#"{"derecha":["privacidad"],"centro":null}"#).is_ok());
        assert!(validar_config_json(r#"{"derecha":[],"centro":"privacidad"}"#).is_ok());
        assert!(validar_config_json(r#"{"derecha":[],"centro":null}"#).is_err());

        assert!(validar_config_json(r#"{"derecha":["relog","privacidad"]}"#).is_err());
        assert!(validar_config_json(r#"{"centro":"relog"}"#).is_err());
        assert!(validar_config_json(r#"{"derecha":["red","red","privacidad"]}"#).is_err());
    }

    /// El cursor y la espera de la captura se validan, y la espera solo admite
    /// los segundos que ofrece la barra.
    // añadido en la 0.2.0
    #[test]
    fn las_opciones_de_captura_se_validan() {
        let valores = validar_config_json(r#"{"captura_cursor":true,"captura_retardo":5}"#)
            .expect("opciones válidas");
        assert!(valores.contains(&("captura_cursor".into(), "si".into())));
        assert!(valores.contains(&("captura_retardo".into(), "5".into())));
        assert!(validar_config_json(r#"{"captura_retardo":4}"#).is_err());
        assert!(validar_config_json(r#"{"captura_retardo":"3"}"#).is_err());
        assert!(validar_config_json(r#"{"captura_cursor":"si"}"#).is_err());
        let json = config_json(&bookos_shell::Config::default());
        assert_eq!(json["captura_cursor"], false);
        assert_eq!(json["captura_retardo"], 0);
    }

    /// `GetConfig` lo devuelve y el cliente lo puede mandar de vuelta tal cual.
    #[test]
    fn el_catalogo_de_widgets_es_de_solo_lectura() {
        let json = config_json(&bookos_shell::Config::default());
        assert_eq!(
            json["widgets_disponibles"].as_array().map(Vec::len),
            Some(bookos_shell::WIDGETS_DISPONIBLES.len())
        );
        let valores = validar_config_json(&json.to_string()).expect("el objeto entero vuelve");
        assert!(!valores.iter().any(|(k, _)| k == "widgets_disponibles"));
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

        let aspecto: String = proxy.call("GetAppearance", &()).expect("GetAppearance");
        let aspecto: serde_json::Value = serde_json::from_str(&aspecto).expect("JSON de aspecto");
        assert_eq!(aspecto["version"], 1);
        assert!(matches!(aspecto["tema"].as_str(), Some("claro" | "oscuro")));
        assert_eq!(aspecto["acento"].as_str().map(str::len), Some(7));

        // El test no es Ajustes ni la app de teclas ni el Player: todo lo que
        // escribe tiene que rechazarse con motivo, y sin llegar al canal (que
        // está cerrado, así que un «no está escuchando» delataría que pasó).
        let (ok, error): (bool, String) = proxy
            .call(
                "ApplyOutputConfig",
                &(Vec::<crate::pantallas::Peticion>::new(),),
            )
            .expect("ApplyOutputConfig");
        assert!(!ok);
        assert!(error.contains("Ajustes"), "{error}");
        let (ok, error): (bool, String) = proxy.call("ApplyConfig", &("{}",)).expect("ApplyConfig");
        assert!(!ok);
        assert!(error.contains("Ajustes"), "{error}");
        let (ok, _): (bool, String) = proxy
            .call("ApplyKeyRemaps", &("[]",))
            .expect("ApplyKeyRemaps");
        assert!(!ok);
        let armada: bool = proxy.call("CaptureKey", &(true,)).expect("CaptureKey");
        assert!(!armada, "cualquiera podía armar la captura de teclas");
        let rechazo = proxy
            .call::<_, _, bool>("PublishActivity", &("com.bookos.player", "player", "{}"))
            .expect_err("cualquiera podía publicar como el Player");
        assert!(
            matches!(&rechazo, zbus::Error::MethodError(_, Some(m), _) if m.contains("com.bookos.player")),
            "el rechazo dice el motivo: {rechazo}"
        );
    }

    #[test]
    fn el_programa_es_el_ejecutable_que_llama() {
        let propio = std::env::current_exe().expect("ruta del test");
        let esperado = propio
            .file_name()
            .and_then(|n| n.to_str())
            .expect("nombre UTF-8");
        assert_eq!(
            programa_de_pid(std::process::id()).as_deref(),
            Some(esperado)
        );
        assert_eq!(programa_de_pid(u32::MAX), None);
    }

    /// Zenith corre como `python3 /usr/bin/bookos-assistant`: el ejecutable es
    /// el intérprete y lo que lo identifica es el script.
    #[test]
    fn con_python_cuenta_el_script() {
        let dir = std::env::temp_dir().join(format!("bookos-comp-prueba-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("carpeta temporal");
        let script = dir.join("bookos-assistant");
        std::fs::write(&script, "import time\ntime.sleep(30)\n").expect("script");
        let Ok(mut hijo) = std::process::Command::new("python3").arg(&script).spawn() else {
            // Sin Python en la máquina de compilación no hay nada que probar.
            return;
        };
        // `spawn` vuelve después del `exec`, así que /proc ya muestra Python.
        let programa = programa_de_pid(hijo.id());
        let _ = hijo.kill();
        let _ = hijo.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(programa.as_deref(), Some("bookos-assistant"));
    }

    #[test]
    fn cada_app_id_de_la_isla_tiene_sus_programas() {
        for app_id in [
            "com.bookos.player",
            "com.bookos.clock",
            "com.bookos.voicerecorder",
            "com.bookos.settings",
            "com.bookos.explorer",
            "com.bookos.calendar",
            "com.bookos.systemmonitor",
            "com.bookos.buds",
            "com.bookos.assistant",
        ] {
            assert!(
                clase_permitida(app_id, "").is_some(),
                "{app_id} fuera de la lista"
            );
            assert!(
                !programas_de(app_id).is_empty(),
                "{app_id} sin programa: nadie podría publicar"
            );
        }
        assert!(programas_de("com.ejemplo.otra").is_empty());
    }
}
