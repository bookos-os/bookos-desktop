//! La batería: porcentaje, si carga y cuánto queda.
//!
//! El pictograma **se dibuja aquí**, no sale del tema de iconos. Es lo mismo
//! que hace el plasmoide `bookos-battery`: cuerpo de 23×11 con borde, relleno
//! proporcional al tanto por ciento y un terminal de 2 px a la derecha. Un icono
//! del tema solo tiene diez escalones y su color lo decide el tema; aquí el
//! relleno es continuo, el color dice el **perfil de energía** —amarillo en
//! ahorro, verde equilibrado, azul rendimiento— y la exclamación de dentro
//! avisa cuando la batería necesita atención. Enchufado a AC no se dibuja la
//! pila: se dibuja un rayo verde en su hueco (ver [`rayo_ac`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use iced_widget::{Space, container, row, text};

use crate::icono::{self, Icono};
use crate::state::Battery;
use crate::tema;
use crate::view::{PELIGRO, PanelElement, TEXT};
use crate::widget::Widget;

/// Medidas del pictograma.
///
/// Parte de las proporciones y el trazo de `battery-*`, pero alarga la carcasa
/// hasta una relación aproximada de 2,4:1, como el diseño del HIG. Así se lee
/// como una batería horizontal y deja suficiente recorrido visible al nivel.
///
/// La caja conserva 18 unidades de alto para alinearse con el resto de iconos
/// del panel; son los símbolos interiores los que deben tener más presencia,
/// no la carcasa completa.
///
/// El dibujo tiene su tinta entre y=6.75 y 18.75, centrada en 12.75.
const VIEWBOX: &str = "0 3.75 30 18";
const ANCHO_ICONO: f32 = 33.0;
const ALTO_ICONO: f32 = 20.0;

/// El color en `#rrggbb`, que es lo que entiende el SVG.
// 0.1.0
fn hex(c: iced_core::Color) -> String {
    let b = c.into_rgba8();
    format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2])
}

/// Qué se dibuja dentro del cuerpo de la batería.
///
/// De dónde come el equipo se dice **dentro** del pictograma y no con el color,
/// que está reservado al perfil de energía. La carga activa usa un rayo y la
/// alimentación directa por el adaptador, cuando la batería ha dejado de
/// cargar, usa un enchufe macizo diseñado para conservarse a 18 px.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Simbolo {
    Nada,
    Rayo,
    Ac,
    Alerta,
}

impl Simbolo {
    // 0.1.0
    fn de(bat: &Battery) -> Self {
        if !bat.plugged && bat.percent <= 20 {
            Self::Alerta
        } else {
            match (bat.charging, bat.plugged) {
                (true, _) => Self::Rayo,
                (false, true) => Self::Ac,
                (false, false) => Self::Nada,
            }
        }
    }

    /// Dónde acaba su trazo más a la derecha, en unidades del `viewBox`.
    ///
    /// Sale de las coordenadas de los `path` de abajo; si se redibuja un
    /// símbolo hay que traerlo aquí, porque es lo que decide el color con el
    /// que se pinta sobre el relleno.
    // 0.1.0
    fn borde_derecho(self) -> f32 {
        match self {
            // Sin símbolo el umbral da igual: no se dibuja nada.
            Self::Nada => 0.0,
            // translate(10.35) + 16 unidades del `bolt` a escala 0,44.
            Self::Rayo => 10.35 + 16.0 * 0.44,
            Self::Ac => 16.95,
            Self::Alerta => 14.88,
        }
    }

    /// Solo para nombrar los SVG que vuelca el test.
    #[cfg(test)]
    fn clave(self) -> &'static str {
        match self {
            Self::Nada => "-",
            Self::Rayo => "rayo",
            Self::Ac => "ac",
            Self::Alerta => "alerta",
        }
    }
}

/// El SVG de la batería con su relleno y su símbolo de alimentación.
///
/// La carcasa conserva el trazo de 1.5 y las esquinas del Heroicon, pero con el
/// cuerpo alargado de la referencia. Lo único que se añade es el relleno, que
/// un icono del tema no puede dar porque solo tiene cuatro escalones y aquí es
/// continuo.
///
/// El `viewBox` va recortado a la caja del dibujo para que el widget no arrastre
/// el aire muerto del `viewBox` de 24: con él, la batería se vería un tercio
/// más pequeña que sus vecinas a igualdad de altura.
// 0.1.0
// modified 0.1.1 -- el dibujo pasa a pictograma_con para poder elegir el contorno
fn pictograma(porciento: u8, simbolo: Simbolo, color: iced_core::Color) -> String {
    pictograma_con(porciento, simbolo, color, crate::view::TEXT())
}

/// El contorno aparte del tema: sobre un fondo de pantalla no vale el color del
/// texto, que con el tema claro es negro.
// 0.1.1
fn pictograma_con(
    porciento: u8,
    simbolo: Simbolo,
    color: iced_core::Color,
    contorno: iced_core::Color,
) -> String {
    // La referencia usa un bloque compacto, casi a toda la altura interior,
    // con muy poco aire respecto a la carcasa. El radio bajo evita que parezca
    // una píldora y hace que se lea como nivel incluso a escala 1x.
    const X0: f32 = 3.35;
    const ANCHO_UTIL: f32 = 21.4;
    let relleno = ANCHO_UTIL * porciento.min(100) as f32 / 100.0;
    let c = hex(color);
    let contorno = hex(contorno);
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{VIEWBOX}">
<rect x="1.5" y="7.5" width="24.75" height="10.5" rx="2.25" fill="none"
      stroke="{contorno}" stroke-opacity="0.72" stroke-width="1.5"/>
<rect x="27.0" y="10.5" width="1.5" height="4.5" rx="0.75" fill="none"
      stroke="{contorno}" stroke-opacity="0.72" stroke-width="1.5"/>
<rect x="{X0}" y="9.35" width="{relleno:.2}" height="6.8" rx="1.0" fill="{c}"/>"##
    );
    // La tinta la decide si el relleno ha llegado a tapar el símbolo ENTERO.
    //
    // Antes el umbral era un 50 % fijo con un comentario que situaba el centro
    // del dibujo en x=11,25. Es falso: el enchufe va de 10,8 a 17,0 y el rayo
    // de 10,35 a 17,39, o sea centrados en 13,9. Con el umbral en 50 el símbolo
    // se pintaba negro ya al 51 %, cuando el relleno solo llegaba a x=14,2 y la
    // mitad derecha quedaba en negro sobre el hueco de la carcasa: sobre un
    // panel oscuro, invisible. El borde derecho de cada símbolo es lo que dice
    // de verdad a partir de qué nivel hay color debajo del último trazo.
    let tinta = if porciento as f32 >= (simbolo.borde_derecho() - X0) / ANCHO_UTIL * 100.0 {
        "#000000".to_string()
    } else {
        contorno.clone()
    };
    match simbolo {
        Simbolo::Nada => {}
        // El rayo es el `bolt` 16/solid del sistema de diseño, encogido al alto
        // del hueco: a 4,5 unidades de alto un trazo no sobrevive —medido, las
        // líneas de 1,5 se comen el hueco— y por eso va macizo y no perfilado.
        Simbolo::Rayo => svg.push_str(&format!(
            r##"<g transform="translate(10.35 9.27) scale(0.44)" fill="{tinta}">
<path fill-rule="evenodd" d="M9.58 1.077a.75.75 0 0 1 .405.82L9.165 6h4.085a.75.75 0 0 1 .567 1.241l-6.5 7.5a.75.75 0 0 1-1.302-.638L6.835 10H2.75a.75.75 0 0 1-.567-1.241l6.5-7.5a.75.75 0 0 1 .897-.182Z" clip-rule="evenodd"/></g>"##
        )),
        // Enchufe macizo: las clavijas anchas y el cuerpo semicircular siguen
        // siendo distinguibles después del rasterizado del panel. El pequeño
        // vástago inferior cuenta que se está usando AC, no que está en pausa.
        Simbolo::Ac => svg.push_str(&format!(
            r##"<g fill="{tinta}">
<rect x="11.45" y="9.3" width="1.05" height="2.65" rx="0.48"/>
<rect x="15.25" y="9.3" width="1.05" height="2.65" rx="0.48"/>
<path d="M10.8 11.2h6.15v.9a3.075 3.075 0 0 1-2.55 3.03v.72a.525.525 0 0 1-1.05 0v-.72a3.075 3.075 0 0 1-2.55-3.03v-.9Z"/>
</g>"##
        )),
        Simbolo::Alerta => svg.push_str(&format!(
            r##"<g fill="{tinta}">
<rect x="12.98" y="9.45" width="1.8" height="4.45" rx="0.7"/>
<circle cx="13.88" cy="15.15" r="1.0"/>
</g>"##
        )),
    }
    svg.push_str("</svg>");
    svg
}

/// Enchufado a AC, el panel no dibuja la pila sino un rayo verde en el mismo
/// hueco: de un vistazo dice «come de la red» mejor que un símbolo diminuto
/// dentro de la carcasa. Mismo `viewBox` que la pila, así que el porcentaje
/// no se mueve al enchufar.
///
/// Es el `bolt` 16/solid del sistema de diseño, centrado en (15, 12.75), que es
/// el centro de la tinta de la pila.
// 0.1.0
fn rayo_ac(color: iced_core::Color) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{VIEWBOX}">
<g transform="translate(7 4.75)" fill="{}">
<path fill-rule="evenodd" d="M9.58 1.077a.75.75 0 0 1 .405.82L9.165 6h4.085a.75.75 0 0 1 .567 1.241l-6.5 7.5a.75.75 0 0 1-1.302-.638L6.835 10H2.75a.75.75 0 0 1-.567-1.241l6.5-7.5a.75.75 0 0 1 .897-.182Z" clip-rule="evenodd"/></g></svg>"##,
        hex(color)
    )
}

/// Medidas de la píldora del panel, en píxeles: el SVG se dibuja 1:1, así que
/// las unidades del `viewBox` son las del texto que va encima.
// añadido en la 0.2.1
const PILDORA_ANCHO: f32 = 33.0;
const PILDORA_ALTO: f32 = 20.0;
const CUERPO_X: f32 = 1.0;
const CUERPO_ANCHO: f32 = 28.5;
const CUERPO_ALTO: f32 = 15.0;
const NUMERO: f32 = 10.5;

/// El rayo que va junto a la píldora mientras carga: el `bolt` 16/solid del
/// sistema de diseño, sin teñir de antemano.
const RAYO_PEQUEÑO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="2 1 12 14" fill="#ffffff"><path fill-rule="evenodd" d="M9.58 1.077a.75.75 0 0 1 .405.82L9.165 6h4.085a.75.75 0 0 1 .567 1.241l-6.5 7.5a.75.75 0 0 1-1.302-.638L6.835 10H2.75a.75.75 0 0 1-.567-1.241l6.5-7.5a.75.75 0 0 1 .897-.182Z" clip-rule="evenodd"/></svg>"##;

/// La píldora del panel, como la del iPhone: pista, relleno hasta el nivel y
/// una punta. **Sin el número**: va encima, en iced, porque el texto de un SVG
/// depende de que `resvg` encuentre una fuente y aquí no se garantiza.
///
/// El relleno se recorta con el cuerpo para que lleve sus mismas esquinas
/// aunque esté a medias. La punta se rellena solo al 100 %.
// añadido en la 0.2.1
fn pastilla_svg(porciento: u8, relleno: iced_core::Color, contorno: iced_core::Color) -> String {
    let ancho = CUERPO_ANCHO * porciento.min(100) as f32 / 100.0;
    let (r, c) = (hex(relleno), hex(contorno));
    let punta = if porciento >= 100 { &r } else { &c };
    let opacidad_punta = if porciento >= 100 { 1.0 } else { 0.4 };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {PILDORA_ANCHO} {PILDORA_ALTO}">
<defs><clipPath id="cuerpo"><rect x="{CUERPO_X}" y="2.5" width="{CUERPO_ANCHO}" height="{CUERPO_ALTO}" rx="4.5"/></clipPath></defs>
<rect x="{CUERPO_X}" y="2.5" width="{CUERPO_ANCHO}" height="{CUERPO_ALTO}" rx="4.5" fill="{c}" fill-opacity="0.28"/>
<rect x="{CUERPO_X}" y="2.5" width="{ancho:.2}" height="{CUERPO_ALTO}" fill="{r}" clip-path="url(#cuerpo)"/>
<rect x="30.4" y="7.5" width="1.8" height="5" rx="0.9" fill="{punta}" fill-opacity="{opacidad_punta}"/>
</svg>"##
    )
}

/// El icono del aviso de conectar o desconectar el cargador: el mismo dibujo
/// que el panel en ese momento, para que la notificación y la barra cuenten lo
/// mismo con el mismo símbolo. Con el icono del tema salía otra batería, de
/// otro estilo, al lado del rayo propio del panel.
///
/// El perfil de energía no se lee: es una llamada a la ACPI (ver
/// [`Bateria::perfil`]) y el color de perfil no aporta nada en un aviso de
/// tres segundos.
// 0.1.0
pub fn icono_aviso(bat: &Battery) -> Icono {
    if bat.plugged {
        return icono::desde_svg(&rayo_ac(tema::verde()));
    }
    icono::desde_svg(&pictograma(
        bat.percent,
        Simbolo::de(bat),
        Bateria::color(bat, None),
    ))
}

/// El widget del panel para pintarlo fuera de él, sobre el fondo de pantalla:
/// la misma pila, con el color del perfil de energía, y la misma cifra al lado.
///
/// Cambian dos cosas. El contorno va en blanco fijo, porque ahí el color del
/// texto del tema puede ser negro. Y enchufado no se cambia la pila por el rayo
/// suelto: en el panel el rayo tiene vecinos que dicen que aquello es la
/// batería, y solo sobre una foto se lee como un adorno. Se deja la pila, en
/// verde, con el rayo dentro.
///
fn leer_perfil_plataforma() -> Option<String> {
    const RUTAS: [&str; 2] = [
        "/sys/firmware/acpi/platform_profile",
        "/sys/class/platform-profile/platform-profile-0/profile",
    ];
    RUTAS.iter().find_map(|ruta| {
        std::fs::read_to_string(ruta)
            .ok()
            .map(|s| s.trim().to_string())
    })
}

fn silencio_adaptativo_activo() -> bool {
    std::fs::read_to_string("/run/bookos-termico/estado")
        .is_ok_and(|s| s.trim() == "balanced-quiet")
}

fn normalizar_perfil_adaptativo(perfil: Option<String>, adaptativo: bool) -> Option<String> {
    if adaptativo
        && matches!(
            perfil.as_deref(),
            Some("low-power" | "quiet" | "power-saver")
        )
    {
        return Some("balanced".to_string());
    }
    perfil
}

fn leer_perfil_efectivo() -> Option<String> {
    normalizar_perfil_adaptativo(leer_perfil_plataforma(), silencio_adaptativo_activo())
}

/// Devuelve el pictograma, su tamaño, la etiqueta y el color de la etiqueta.
// 0.1.1
pub fn sobre_fondo(bat: &Battery) -> (Icono, (f32, f32), String, iced_core::Color) {
    let blanco = iced_core::Color::WHITE;
    let color = if bat.plugged {
        tema::verde()
    } else {
        let perfil = leer_perfil_efectivo();
        match Bateria::color(bat, perfil.as_deref().map(str::trim)) {
            // Sin perfil conocido el panel usa el color del texto del tema.
            color if color == TEXT() => blanco,
            color => color,
        }
    };
    let icono = icono::desde_svg(&pictograma_con(
        bat.percent,
        Simbolo::de(bat),
        color,
        blanco,
    ));
    let tinta = if bat.percent <= 15 && !bat.plugged {
        PELIGRO()
    } else {
        blanco
    };
    (
        icono,
        (ANCHO_ICONO, ALTO_ICONO),
        Bateria::etiqueta(bat),
        tinta,
    )
}

/// Qué enseña el widget. Globales y no campos porque `panel.conf` lo decide
/// Ajustes y los widgets se construyen sin acceso a la configuración.
// añadido en la 0.2.2
static VER_PORCENTAJE: AtomicBool = AtomicBool::new(true);
static VER_TIEMPO: AtomicBool = AtomicBool::new(true);
/// Con colores, el relleno dice el perfil de energía; sin ellos (el icono
/// sólido) va en el color del texto y solo se colorea lo que pide atención:
/// cargando, batería baja y modo ahorro.
// añadido en la 0.2.2
static COLORES_PERFIL: AtomicBool = AtomicBool::new(true);
/// El perfil cambió y la caché de [`CADA_PERFIL`] ya no vale. Lo pone el
/// compositor al ver el aviso de sysfs en `platform_profile`.
// añadido en la 0.2.2
static PERFIL_SUCIO: AtomicBool = AtomicBool::new(false);

/// El perfil de energía ha cambiado: el siguiente refresco lo relee.
// añadido en la 0.2.2
pub fn perfil_cambiado() {
    PERFIL_SUCIO.store(true, Ordering::Relaxed);
}

/// Aplica lo que dice `panel.conf` sobre porcentaje y tiempo restante.
// añadido en la 0.2.2
pub fn mostrar(porcentaje: bool, tiempo: bool, colores: bool) {
    VER_PORCENTAJE.store(porcentaje, Ordering::Relaxed);
    VER_TIEMPO.store(tiempo, Ordering::Relaxed);
    COLORES_PERFIL.store(colores, Ordering::Relaxed);
}

pub struct Bateria {
    dato: Option<Battery>,
    /// El icono que toca para el nivel y el estado de carga actuales.
    icono: Option<Icono>,
    /// Con qué nombre se cargó, para no volver a buscarlo mientras no cambie.
    icono_nombre: String,
    /// El perfil de energía, que decide el color cuando no manda el nivel.
    perfil: Option<String>,
    /// Cuándo se leyó por última vez cada una de las dos cosas caras. Ver
    /// [`CADA_PERFIL`] y [`CADA_MINUTOS`].
    perfil_leido: Instant,
    minutos_leidos: Instant,
    /// Evita pedir `power-saver` en cada aviso de udev mientras siga baja.
    ahorro_automatico: bool,
    /// Cuándo cruzó el 15 %, para los dos latidos de la cifra.
    alerta_desde: Option<Instant>,
    /// El rayo de al lado de la píldora, cargado una vez.
    // añadido en la 0.2.1
    rayo: Icono,
    /// Con qué ajustes de [`mostrar`] se pintó: porcentaje, tiempo y colores.
    /// Cambiarlos desde Ajustes no trae ningún dato nuevo de la batería, así
    /// que sin compararlos el refresco decía «nada cambió» y la pila se
    /// quedaba con el relleno de antes hasta el siguiente cambio de perfil.
    // añadido en la 0.2.2
    ajustes: (bool, bool, bool),
}

/// Los ajustes de [`mostrar`] ahora mismo.
// añadido en la 0.2.2
fn ajustes_vista() -> (bool, bool, bool) {
    (
        VER_PORCENTAJE.load(Ordering::Relaxed),
        VER_TIEMPO.load(Ordering::Relaxed),
        COLORES_PERFIL.load(Ordering::Relaxed),
    )
}

/// Cada cuánto se vuelven a leer las dos fuentes lentas.
///
/// **Medido en esta máquina**: `platform_profile` cuesta 0,71 ms y
/// `current_now` 0,53, contra los 0,02 de `capacity` y `status`. Las dos son
/// consultas al firmware, no lecturas de un fichero, y se pagaban enteras en
/// cada evento de `power_supply` —que llegan de varios en varios.
///
/// Los intervalos son los de lo que enseñan, no números redondos: el perfil
/// solo cambia cuando alguien lo cambia a mano, y los minutos restantes salen
/// de una corriente media de dos minutos y se enseñan al minuto, así que
/// leerlos más de una vez cada medio minuto no cambia ni un píxel.
const CADA_PERFIL: Duration = Duration::from_secs(5);
/// Un latido del aviso de batería crítica, y cuántos da.
const D_ALERTA: Duration = Duration::from_millis(900);
const LATIDOS: u32 = 2;
const CADA_MINUTOS: Duration = Duration::from_secs(30);

impl Bateria {
    // 0.1.0
    pub fn new() -> Self {
        let mut b = Self {
            dato: None,
            icono: None,
            icono_nombre: String::new(),
            perfil: None,
            // Restados, para que el primer refresco —el del arranque— lea las
            // dos: el panel tiene que salir con su color y su tiempo puestos.
            perfil_leido: Instant::now() - CADA_PERFIL,
            minutos_leidos: Instant::now() - CADA_MINUTOS,
            ahorro_automatico: false,
            alerta_desde: None,
            rayo: icono::desde_svg(RAYO_PEQUEÑO),
            ajustes: ajustes_vista(),
        };
        b.refrescar();
        b
    }

    /// Rehace el pictograma si cambió algo de lo que dibuja.
    ///
    /// El SVG se regenera solo cuando cambia el escalón entero o el color:
    /// rasterizarlo en cada lectura sería trabajo por nada, y el relleno no se
    /// distingue por debajo de un punto porcentual.
    // 0.1.0
    // modified 0.2.1 -- la píldora con el número dentro, también enchufado
    fn actualizar_icono(&mut self) -> bool {
        let Some(bat) = self.dato else {
            let habia = self.icono.take().is_some();
            self.icono_nombre.clear();
            return habia;
        };
        let relleno = self.relleno(&bat);
        let clave = format!("{}-{}-{}", bat.percent, hex(relleno), hex(TEXT()));
        if clave == self.icono_nombre {
            return false;
        }
        self.icono = Some(icono::desde_svg(&pastilla_svg(
            bat.percent,
            relleno,
            TEXT(),
        )));
        self.icono_nombre = clave;
        true
    }

    /// El color del relleno: verde mientras carga o enchufado; si no, el que
    /// dice [`Bateria::color`] (el del perfil de energía, o el rojo del aviso).
    /// Sin nada que decir va en la tinta del tema.
    // añadido en la 0.2.1
    fn relleno(&self, bat: &Battery) -> iced_core::Color {
        if bat.charging || bat.plugged {
            tema::verde()
        } else {
            Self::color(bat, self.perfil.as_deref())
        }
    }

    /// Lo que va fuera de la píldora: el tiempo restante, si lo hay. El
    /// porcentaje ya está dentro.
    // añadido en la 0.2.1
    fn tiempo(bat: &Battery) -> Option<String> {
        // Mismo criterio que antes: a punto de llegar al tope, «0:04» es ruido.
        bat.minutes
            .filter(|_| VER_TIEMPO.load(Ordering::Relaxed))
            .filter(|m| !bat.charging || *m >= 5)
            .map(|min| format!("{}:{:02}", min / 60, min % 60))
    }

    /// Activa ahorro al entrar en el tramo bajo, una vez por descarga.
    ///
    /// `busctl` no corre en el hilo del compositor: aunque D-Bus tarde, el
    /// cursor y las animaciones siguen respondiendo. Al conectar corriente o
    /// superar el 20 % se rearma, pero no se cambia de vuelta el perfil que
    /// haya elegido el usuario.
    // 0.1.0
    fn revisar_ahorro_automatico(&mut self, bat: Option<&Battery>) {
        let baja = bat.is_some_and(|b| !b.plugged && b.percent <= 20);
        if !baja {
            self.ahorro_automatico = false;
            return;
        }
        if self.ahorro_automatico
            || matches!(
                self.perfil.as_deref(),
                Some("low-power" | "power-saver" | "quiet")
            )
        {
            return;
        }
        self.ahorro_automatico = true;
        std::thread::Builder::new()
            .name("bookos-battery-saver".into())
            .spawn(|| {
                let resultado = std::process::Command::new("busctl")
                    .args([
                        "--system",
                        "set-property",
                        "net.hadess.PowerProfiles",
                        "/net/hadess/PowerProfiles",
                        "net.hadess.PowerProfiles",
                        "ActiveProfile",
                        "s",
                        "power-saver",
                    ])
                    .status();
                if !resultado.is_ok_and(|s| s.success()) {
                    tracing::warn!("no se pudo activar el ahorro automático de batería");
                }
            })
            .inspect_err(|err| tracing::warn!("no se pudo iniciar el ahorro automático: {err}"))
            .ok();
    }

    /// Rellena el tiempo restante en los refrescos que no lo leen.
    ///
    /// La media de la corriente ya la hace `state` (ver `flujo_medio`), que es
    /// la misma para el panel, el emergente y el buscador. Aquí solo queda no
    /// perder la cifra entre dos lecturas caras: sin esto, los refrescos que
    /// solo miran el porcentaje dejarían `minutes` en `None` y la etiqueta
    /// perdería el «2:15» hasta la siguiente lectura, un parpadeo cada pocos
    /// segundos.
    // 0.1.0
    fn conservar_minutos(&self, fresco: &mut Battery, leido: bool) {
        // Enchufar o desenchufar cambia el signo de lo que se mide: la cifra
        // anterior no vale. El evento de udev del cargador llega casi siempre
        // en un refresco sin lectura, y si ahí se copiaba, el panel seguía con
        // la autonomía de descarga (1:35 en vez de ~0:30).
        if leido {
            return;
        }
        fresco.minutes = self
            .dato
            .filter(|b| b.charging == fresco.charging)
            .and_then(|b| b.minutes);
    }
}

impl Bateria {
    /// El color del pictograma: **el del perfil de energía**.
    ///
    /// Antes mandaba el estado de carga —verde cargando, ámbar por debajo del
    /// 30 %— y el perfil solo se veía el resto del tiempo, que en un portátil
    /// enchufado es casi nunca. Ahora la alimentación la cuenta el símbolo de
    /// dentro (rayo, AC o alerta) y el color queda libre para lo único que no
    /// tiene otro sitio donde enseñarse: en qué perfil va el equipo.
    ///
    /// La única excepción es el rojo bajo mínimos y desenchufado, que es un
    /// aviso y no un estado: ahí el perfil da igual.
    // 0.1.0
    fn color(bat: &Battery, perfil: Option<&str>) -> iced_core::Color {
        Self::color_con(bat, perfil, COLORES_PERFIL.load(Ordering::Relaxed))
    }

    /// La regla de [`Bateria::color`] con el estilo dado, para poder probarla
    /// sin tocar el ajuste global que comparten los tests en paralelo.
    // añadido en la 0.2.2
    fn color_con(bat: &Battery, perfil: Option<&str>, colores: bool) -> iced_core::Color {
        let ahorro = matches!(perfil, Some("low-power" | "power-saver" | "quiet"));
        if bat.percent <= 15 && !bat.plugged {
            PELIGRO()
        } else if !colores {
            if bat.percent <= 20 && !bat.plugged {
                tema::bateria_baja()
            } else if ahorro {
                tema::perfil_ahorro()
            } else {
                TEXT()
            }
        } else if bat.percent <= 20 && !bat.plugged {
            tema::perfil_ahorro()
        } else {
            // Cada perfil con su color, que son los del diseño: ahorro
            // amarillo, equilibrado verde y rendimiento azul. `low-power` es
            // como lo llama la ACPI y `power-saver` como lo llama
            // power-profiles-daemon; el fichero de sysfs trae uno u otro según
            // el driver de la plataforma.
            match perfil {
                Some("low-power" | "power-saver" | "quiet") => tema::perfil_ahorro(),
                Some("balanced" | "balanced-performance") => tema::perfil_equilibrado(),
                Some("performance") => tema::perfil_rendimiento(),
                _ => TEXT(),
            }
        }
    }

    /// El perfil de energía, leído de sysfs.
    ///
    /// Se lee el fichero en vez de preguntar a power-profiles-daemon por D-Bus
    /// porque esto ocurre en el refresco del panel: `busctl` es un proceso
    /// nuevo cada vez. Lo que **no** es cierto es que leerlo sea barato:
    /// `platform_profile` no es un fichero, es una llamada a la ACPI, y medido
    /// en esta máquina cuesta **0,71 ms** —treinta veces más que `capacity`—.
    ///
    /// Por eso no se lee en cada refresco sino como mucho cada [`CADA_PERFIL`]:
    /// el perfil solo cambia cuando alguien lo cambia a mano, y los refrescos
    /// llegan en ráfaga con cada evento de `power_supply`.
    // 0.1.0
    fn perfil(&mut self) -> Option<String> {
        if !PERFIL_SUCIO.swap(false, Ordering::Relaxed) && self.perfil_leido.elapsed() < CADA_PERFIL
        {
            return self.perfil.clone();
        }
        self.perfil_leido = Instant::now();
        leer_perfil_efectivo()
    }

    /// Lo que se enseña al lado del icono.
    ///
    /// Solo el número: lo demás lo cuenta el icono, que lleva el rayo cuando
    /// carga, y el color, que se pone verde cargando y rojo por debajo del
    /// 15 %. "Cargando 80%" al lado de un icono con un rayo es decir lo mismo
    /// dos veces y ocupa el ancho de otro estado entero.
    ///
    /// Está aparte de `ver` porque `ancho` necesita contar sus caracteres, y
    /// dos copias de esta regla se desincronizan a la primera.
    // 0.1.0
    fn etiqueta(bat: &crate::state::Battery) -> String {
        // Con espacio, como se escribe el tanto por ciento en castellano, y
        // como lo escribe el diseño («67 %»).
        let mut etiqueta = format!("{} %", bat.percent);
        // El tiempo solo aparece cuando la cuenta sale y aún tiene sentido
        // enseñarla: a punto de llegar al tope, "0:04" es ruido. Se mira el
        // tiempo y no el porcentaje porque el tope puede ser 80 y no 100.
        if let Some(min) = bat.minutes.filter(|m| !bat.charging || *m >= 5) {
            etiqueta.push_str(&format!(" {}:{:02}", min / 60, min % 60));
        }
        etiqueta
    }
}

impl Widget for Bateria {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "bateria"
    }

    fn accesible(&self) -> (&'static str, String) {
        (crate::emergente::t("Batería", "Battery"), crate::widget::bateria_accesible(self.dato.as_ref()))
    }

    // 0.1.0
    fn subsistemas(&self) -> &'static [&'static str] {
        &["power_supply"]
    }

    // 0.1.0
    fn refrescar(&mut self) -> bool {
        // El tiempo restante solo se estima de tanto en tanto: es la parte que
        // le cuesta medio milisegundo al controlador embebido.
        let toca_minutos = self.minutos_leidos.elapsed() >= CADA_MINUTOS;
        if toca_minutos {
            self.minutos_leidos = Instant::now();
        }
        let mut fresco = Battery::read(toca_minutos);
        if let Some(b) = fresco.as_mut() {
            self.conservar_minutos(b, toca_minutos);
        }
        let perfil = self.perfil();
        let ajustes = ajustes_vista();
        if fresco == self.dato && perfil == self.perfil && ajustes == self.ajustes {
            // La clave del icono lleva la tinta del tema: sin esto, al pasar a
            // claro u oscuro la píldora seguía con la tinta vieja hasta que
            // la batería variase un punto.
            return self.actualizar_icono();
        }
        self.ajustes = ajustes;
        let critica = |b: &Option<Battery>| b.is_some_and(|b| b.percent <= 15 && !b.plugged);
        if critica(&fresco) && !critica(&self.dato) && !tema::efectos_reducidos() {
            self.alerta_desde = Some(Instant::now());
        }
        self.dato = fresco;
        self.perfil = perfil;
        let dato = self.dato;
        self.revisar_ahorro_automatico(dato.as_ref());
        self.actualizar_icono();
        true
    }

    // 0.1.0
    fn animando(&self) -> bool {
        self.alerta_desde
            .is_some_and(|t| t.elapsed() < D_ALERTA * LATIDOS)
    }

    /// La píldora, el rayo si carga y el tiempo restante si lo hay.
    // 0.1.0
    // modified 0.2.1 -- el porcentaje va dentro de la píldora
    fn ancho(&self) -> f32 {
        let Some(bat) = &self.dato else {
            return 0.0;
        };
        let rayo = if bat.charging || bat.plugged {
            RAYO_LADO + 3.0
        } else {
            0.0
        };
        let tiempo =
            Self::tiempo(bat).map_or(0.0, |t| 5.0 + crate::widget::ancho_de(&t, tema::T_CUERPO));
        rayo + PILDORA_ANCHO + tiempo
    }

    // 0.1.0
    // modified 0.2.1 -- píldora con el número dentro, que se invierte donde lo cubre el relleno
    fn ver(&self) -> PanelElement<'_> {
        let Some(bat) = &self.dato else {
            return crate::widget::vacio();
        };
        // Al cruzar el 15 % la cifra late dos veces y se queda quieta. Latir
        // siempre sería un reclamo que no se puede apagar mientras se busca el
        // cargador.
        let mut pulso = 1.0;
        if let Some(desde) = self.alerta_desde {
            let pasado = desde.elapsed();
            if pasado < D_ALERTA * LATIDOS {
                let ciclo =
                    (pasado.as_secs_f32() % D_ALERTA.as_secs_f32()) / D_ALERTA.as_secs_f32();
                pulso = 1.0 - 0.55 * (std::f32::consts::PI * ciclo).sin();
            }
        }
        let numero = bat.percent.min(100).to_string();
        let fuente = iced_core::Font {
            weight: iced_core::font::Weight::Bold,
            ..iced_core::Font::DEFAULT
        };
        // Las dos capas de texto salen del mismo sitio, por la misma cuenta: la
        // de abajo con la tinta del tema sobre la pista, y la de arriba, con la
        // tinta que se lea sobre el relleno, recortada a donde llega el
        // relleno. Donde el relleno cubre el número, este se invierte; así se
        // lee igual a cualquier carga.
        let x = CUERPO_X
            + (CUERPO_ANCHO - crate::widget::ancho_con_fuente(&numero, NUMERO, fuente)) / 2.0;
        let capa = |color: iced_core::Color| {
            let mut color = color;
            color.a *= pulso;
            container(text(numero.clone()).size(NUMERO).font(fuente).color(color))
                .padding(iced_core::Padding::ZERO.left(x))
                .height(iced_core::Length::Fixed(PILDORA_ALTO))
                .center_y(iced_core::Length::Fixed(PILDORA_ALTO))
        };
        let relleno = self.relleno(bat);
        let hasta = CUERPO_X + CUERPO_ANCHO * bat.percent.min(100) as f32 / 100.0;
        let mut pastilla = iced_widget::stack![];
        if let Some(ic) = &self.icono {
            // Sin teñir: el pictograma ya trae dentro los dos colores.
            pastilla = pastilla.push(icono::ver(ic, PILDORA_ANCHO, PILDORA_ALTO));
        }
        if VER_PORCENTAJE.load(Ordering::Relaxed) {
            pastilla = pastilla
                .push(capa(TEXT()))
                .push(crate::recorte::Recorte::new(
                    capa(tema::tinta_sobre(relleno)),
                    hasta,
                    PILDORA_ALTO,
                    0.0,
                ));
        }
        let pastilla = pastilla
            .width(iced_core::Length::Fixed(PILDORA_ANCHO))
            .height(iced_core::Length::Fixed(PILDORA_ALTO));

        let mut fila = row![]
            .spacing(3)
            .align_y(iced_core::alignment::Vertical::Center);
        if bat.charging || bat.plugged {
            fila = fila.push(icono::ver_teñido(
                &self.rayo,
                RAYO_LADO,
                Some(tema::verde()),
            ));
        }
        fila = fila.push(pastilla);
        if let Some(tiempo) = Self::tiempo(bat) {
            fila = fila.push(Space::new().width(iced_core::Length::Fixed(2.0)));
            let mut color = TEXT();
            color.a *= pulso;
            fila = fila.push(text(tiempo).size(tema::T_CUERPO).color(color));
        }
        fila.into()
    }
}

/// El lado de la caja del rayo de al lado de la píldora.
const RAYO_LADO: f32 = 12.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: u8, charging: bool, plugged: bool) -> Battery {
        Battery {
            percent,
            charging,
            plugged,
            minutes: None,
        }
    }

    /// Cambiar el estilo en Ajustes no trae datos nuevos de la batería: el
    /// refresco tiene que darse cuenta igual, o la pila se queda con el
    /// relleno de antes hasta el siguiente cambio de perfil.
    #[test]
    fn cambiar_los_ajustes_rehace_la_pila() {
        let mut w = Bateria::new();
        w.refrescar();
        let (p, t, c) = ajustes_vista();
        w.ajustes = (p, t, !c);
        // Sin batería en la máquina de pruebas no hay pila que rehacer.
        if w.dato.is_some() {
            assert!(w.refrescar(), "otro estilo es otra pila");
        }
        assert_eq!(w.ajustes, ajustes_vista());
    }

    /// Enchufar entre dos lecturas de corriente no puede dejar en el panel la
    /// autonomía de descarga como si fuera el tiempo de carga.
    #[test]
    fn al_enchufar_no_se_arrastra_la_autonomia() {
        let mut w = Bateria::new();
        let mut descarga = Battery {
            minutes: Some(95),
            ..bat(67, false, false)
        };
        w.conservar_minutos(&mut descarga, true);
        w.dato = Some(descarga);

        // El evento del cargador, sin lectura cara.
        let mut enchufado = bat(67, true, true);
        w.conservar_minutos(&mut enchufado, false);
        assert_eq!(enchufado.minutes, None);
        w.dato = Some(enchufado);

        let mut carga = Battery {
            minutes: Some(31),
            ..bat(67, true, true)
        };
        w.conservar_minutos(&mut carga, true);
        assert_eq!(carga.minutes, Some(31));
    }

    /// El símbolo solo se pinta en negro cuando el relleno lo tapa ENTERO.
    ///
    /// Con el umbral fijo del 50 % que había antes, un 51 % pintaba el enchufe
    /// en negro cuando el relleno solo llegaba a x=14,2 de los 17,0 que ocupa:
    /// el último tercio quedaba negro sobre el hueco de la carcasa, o sea
    /// invisible en el panel oscuro. `borde_derecho` es lo que fija la frontera.
    ///
    /// Con `BOOKOS_BATERIA_SVG=/ruta` deja los pictogramas de estos niveles en
    /// disco, que es la forma de mirarlos sin una batería de verdad delante.
    #[test]
    fn el_simbolo_no_se_pinta_en_negro_hasta_que_el_relleno_lo_tapa() {
        let destino = std::env::var("BOOKOS_BATERIA_SVG").ok();
        if let Some(d) = &destino {
            std::fs::create_dir_all(d).expect("el directorio de volcado");
        }
        for (simbolo, umbral) in [
            (Simbolo::Ac, 64u8),
            (Simbolo::Rayo, 66),
            (Simbolo::Alerta, 54),
        ] {
            for nivel in [5, 35, 50, umbral - 1, umbral, 95] {
                let svg = pictograma(nivel, simbolo, tema::acento());
                let negro = svg.contains(r##"fill="#000000""##);
                assert_eq!(
                    negro,
                    nivel >= umbral,
                    "{} al {nivel} %: negro={negro}, y el relleno llega a x={:.1} de {:.1}",
                    simbolo.clave(),
                    3.35 + 21.4 * nivel as f32 / 100.0,
                    simbolo.borde_derecho(),
                );
                if let Some(d) = &destino {
                    let _ = std::fs::write(format!("{d}/{}-{nivel}.svg", simbolo.clave()), &svg);
                }
            }
        }
    }

    /// El perfil manda sobre el estado de carga: enchufado y cargando en modo
    /// ahorro, el pictograma sigue siendo amarillo. Antes se ponía verde y el
    /// perfil no se veía nunca en un portátil de sobremesa.
    #[test]
    fn el_color_es_el_del_perfil() {
        assert_eq!(
            Bateria::color(&bat(80, true, true), Some("low-power")),
            tema::perfil_ahorro()
        );
        assert_eq!(
            Bateria::color(&bat(80, false, true), Some("balanced")),
            tema::perfil_equilibrado()
        );
        assert_eq!(
            Bateria::color(&bat(80, false, false), Some("performance")),
            tema::perfil_rendimiento()
        );
        assert_eq!(Bateria::color(&bat(80, false, false), None), TEXT());
    }

    /// Bajo mínimos y desenchufado el rojo se impone: es un aviso, no un
    /// estado. Enchufado al 10 % no hay nada que avisar.
    #[test]
    fn el_rojo_solo_avisa_sin_corriente() {
        assert_eq!(
            Bateria::color(&bat(10, false, false), Some("balanced")),
            PELIGRO()
        );
        assert_eq!(
            Bateria::color(&bat(10, false, true), Some("balanced")),
            tema::perfil_equilibrado()
        );
        assert_eq!(
            Bateria::color(&bat(20, false, false), Some("performance")),
            tema::perfil_ahorro(),
            "el tramo 16-20 fuerza el amarillo de ahorro"
        );
    }

    /// El rayo y el enchufe distinguen carga activa de alimentación directa
    /// por AC cuando el límite de carga ha detenido la batería.
    #[test]
    fn el_rayo_solo_aparece_mientras_carga() {
        assert!(Simbolo::de(&bat(80, true, true)) == Simbolo::Rayo);
        assert!(Simbolo::de(&bat(80, false, true)) == Simbolo::Ac);
        assert!(Simbolo::de(&bat(80, false, false)) == Simbolo::Nada);
        assert!(Simbolo::de(&bat(20, false, false)) == Simbolo::Alerta);
        assert!(Simbolo::de(&bat(15, false, false)) == Simbolo::Alerta);
    }

    /// El SVG generado tiene que llevar el dibujo que toca. Es lo único que
    /// distingue la carga activa, y se genera con `format!`.
    #[test]
    fn el_pictograma_dibuja_su_simbolo() {
        // El rayo del sistema de diseño va en un grupo con su escala.
        let rayo = pictograma(80, Simbolo::Rayo, tema::acento());
        assert!(
            rayo.contains("<g transform"),
            "el rayo va escalado en un grupo"
        );
        assert_eq!(
            rayo.matches("<rect").count(),
            3,
            "carcasa, terminal y relleno"
        );

        let nada = pictograma(80, Simbolo::Nada, tema::acento());
        assert_eq!(nada.matches("<rect").count(), 3);

        let ac = pictograma(80, Simbolo::Ac, tema::acento());
        assert!(!ac.contains("<g transform"));
        assert_eq!(ac.matches("<rect").count(), 5);
        let alerta = pictograma(20, Simbolo::Alerta, tema::perfil_ahorro());
        assert!(alerta.contains("<circle"));
    }

    /// El relleno es continuo y proporcional: es lo que un icono del tema, con
    /// sus cuatro escalones, no puede dar.
    #[test]
    fn el_solido_solo_colorea_lo_que_avisa() {
        let c =
            |p: u8, perfil: &str| Bateria::color_con(&bat(p, false, false), Some(perfil), false);
        assert_eq!(c(80, "balanced"), TEXT());
        assert_eq!(c(80, "performance"), TEXT());
        assert_eq!(c(80, "power-saver"), tema::perfil_ahorro());
        assert_eq!(c(18, "balanced"), tema::bateria_baja());
        assert_eq!(c(6, "balanced"), PELIGRO());
        // Con colores, el perfil manda.
        let c = |perfil: &str| Bateria::color_con(&bat(80, false, false), Some(perfil), true);
        assert_eq!(c("balanced"), tema::perfil_equilibrado());
        assert_eq!(c("performance"), tema::perfil_rendimiento());
        assert_eq!(c("low-power"), tema::perfil_ahorro());
    }

    #[test]
    fn el_relleno_sigue_al_porcentaje() {
        // Se busca el rectángulo por su origen para no confundirlo con la
        // carcasa ni con el terminal.
        let ancho = |p: u8| {
            let svg = pictograma(p, Simbolo::Nada, tema::acento());
            let rect = &svg[svg.find("<rect x=\"3.35\"").unwrap()..];
            let i = rect.find("width=\"").unwrap() + 7;
            let resto = &rect[i..];
            resto[..resto.find('"').unwrap()].parse::<f32>().unwrap()
        };
        assert_eq!(ancho(0), 0.0);
        assert!((ancho(50) - 10.7).abs() < 0.01, "la mitad del hueco");
        assert!((ancho(100) - 21.4).abs() < 0.01, "el hueco entero");
        // Un dato imposible no desborda la carcasa.
        assert_eq!(ancho(200), ancho(100));
    }

    /// El símbolo se pinta en negro solo cuando el relleno le ha llegado por
    /// debajo: en blanco sobre el color no se distinguiría.
    #[test]
    fn la_tinta_del_simbolo_depende_del_relleno() {
        assert!(pictograma(80, Simbolo::Rayo, tema::acento()).contains("#000000"));
        assert!(!pictograma(20, Simbolo::Rayo, tema::acento()).contains("#000000"));
    }

    #[test]
    fn el_silencio_adaptativo_muestra_el_perfil_equilibrado() {
        assert_eq!(
            normalizar_perfil_adaptativo(Some("quiet".into()), true).as_deref(),
            Some("balanced")
        );
        assert_eq!(
            normalizar_perfil_adaptativo(Some("low-power".into()), true).as_deref(),
            Some("balanced")
        );
        assert_eq!(
            normalizar_perfil_adaptativo(Some("low-power".into()), false).as_deref(),
            Some("low-power")
        );
        assert_eq!(
            normalizar_perfil_adaptativo(Some("performance".into()), true).as_deref(),
            Some("performance")
        );
    }
}
