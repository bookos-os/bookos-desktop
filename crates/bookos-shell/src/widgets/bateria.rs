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

use std::time::{Duration, Instant};

use iced_widget::{row, text};

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

    /// Para la clave del icono cacheado.
    // 0.1.0
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
/// Devuelve el pictograma, su tamaño, la etiqueta y el color de la etiqueta.
// 0.1.1
pub fn sobre_fondo(bat: &Battery) -> (Icono, (f32, f32), String, iced_core::Color) {
    let blanco = iced_core::Color::WHITE;
    let color = if bat.plugged {
        tema::verde()
    } else {
        let perfil = std::fs::read_to_string("/sys/firmware/acpi/platform_profile").ok();
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
    fn actualizar_icono(&mut self) {
        let Some(bat) = self.dato else {
            self.icono = None;
            self.icono_nombre.clear();
            return;
        };
        if bat.plugged {
            let clave = "ac".to_string();
            if clave != self.icono_nombre {
                self.icono = Some(icono::desde_svg(&rayo_ac(tema::verde())));
                self.icono_nombre = clave;
            }
            return;
        }
        let color = Self::color(&bat, self.perfil.as_deref());
        let simbolo = Simbolo::de(&bat);
        let clave = format!("{}-{}-{}", bat.percent, simbolo.clave(), hex(color));
        if clave == self.icono_nombre {
            return;
        }
        self.icono = Some(icono::desde_svg(&pictograma(bat.percent, simbolo, color)));
        self.icono_nombre = clave;
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
        if bat.percent <= 15 && !bat.plugged {
            PELIGRO()
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
        if self.perfil_leido.elapsed() < CADA_PERFIL {
            return self.perfil.clone();
        }
        self.perfil_leido = Instant::now();
        std::fs::read_to_string("/sys/firmware/acpi/platform_profile")
            .ok()
            .map(|s| s.trim().to_string())
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
        if fresco == self.dato && perfil == self.perfil {
            return false;
        }
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

    /// Icono, su hueco y el texto, que puede ser "85 %" o "85 % 2:15".
    // 0.1.0
    fn ancho(&self) -> f32 {
        let Some(bat) = &self.dato else {
            return 0.0;
        };
        ANCHO_ICONO + 5.0 + crate::widget::ancho_de(&Self::etiqueta(bat), tema::T_CUERPO)
    }

    // 0.1.0
    fn ver(&self) -> PanelElement<'_> {
        let Some(bat) = &self.dato else {
            return crate::widget::vacio();
        };
        // El texto **no** sigue al color del pictograma: en el diseño el "67 %"
        // va en blanco con la batería amarilla, porque quien indica el estado
        // es el dibujo y el número solo es el dato. Solo bajo mínimos se pinta
        // también el texto, que ahí sí conviene que grite.
        let mut color = if bat.percent <= 15 && !bat.plugged {
            PELIGRO()
        } else {
            TEXT()
        };
        // Al cruzar el 15 % la cifra late dos veces y se queda quieta. Latir
        // siempre sería un reclamo que no se puede apagar mientras se busca el
        // cargador.
        if let Some(desde) = self.alerta_desde {
            let pasado = desde.elapsed();
            if pasado < D_ALERTA * LATIDOS {
                let ciclo =
                    (pasado.as_secs_f32() % D_ALERTA.as_secs_f32()) / D_ALERTA.as_secs_f32();
                color.a *= 1.0 - 0.55 * (std::f32::consts::PI * ciclo).sin();
            }
        }
        let etiqueta = Self::etiqueta(bat);

        let mut fila = row![]
            .spacing(5)
            .align_y(iced_core::alignment::Vertical::Center);
        if let Some(ic) = &self.icono {
            // Sin teñir: el pictograma ya trae dentro el color del estado, el
            // del borde y el del relleno, que son tres y distintos.
            fila = fila.push(icono::ver(ic, ANCHO_ICONO, ALTO_ICONO));
        }
        fila.push(text(etiqueta).size(tema::T_CUERPO).color(color))
            .into()
    }
}

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
}
