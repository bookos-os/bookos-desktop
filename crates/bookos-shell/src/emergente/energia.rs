//! El emergente de la batería: cuánto queda y en qué perfil va el equipo.
//!
//! ## Por qué el perfil se pide por D-Bus y no se escribe en sysfs
//!
//! `/sys/firmware/acpi/platform_profile` es de root con permisos 644, igual que
//! el brillo. Quien concede el permiso es `power-profiles-daemon`, que expone
//! `net.hadess.PowerProfiles.ActiveProfile` como propiedad escribible y además
//! sabe combinar el perfil de la plataforma con el gobernador de la CPU —aquí,
//! `platform_profile` con `intel_pstate`—; escribiendo el fichero a pelo solo
//! se movería la mitad.
//!
//! Se llama con `busctl`, como en el brillo: meter un cliente de D-Bus sería
//! una dependencia nueva y un hilo más dentro del compositor para escribir una
//! cadena. La llamada de escritura se lanza sin esperar (`spawn`), que es lo
//! mismo que se hizo con `wpctl` cuando se midió que bloquear costaba 21 ms.
//!
//! Los perfiles que existen los dice el propio daemon, no una lista escrita
//! aquí: hay equipos con tres y equipos con cuatro.

use std::process::{Command, Stdio};

use iced_core::alignment::Vertical;
use iced_core::border::Radius;
use iced_core::font::Weight;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, stack, text};

use crate::Accion;
use crate::icono::{self, Icono};
use crate::state::Battery;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, ES, Tecla, t};
use super::{control, lista};

// Las medidas del lienzo «Panel de batería BookOS», versión normal: tarjeta de
// 336 con margen de 16; cabecera con el nivel, la barra y el renglón de estado;
// los perfiles en una lista segmentada, y el pie.
const ANCHO: f32 = 336.0;
const MARGEN: f32 = 16.0;
const INTERIOR: f32 = ANCHO - MARGEN * 2.0;
/// Fila del título «Batería» con la pastilla de estado a la derecha.
const TITULO: f32 = 22.0;
const BAJO_TITULO: f32 = 4.0;
/// El porcentaje, a 28/Bold: el «Título app» del sistema de diseño.
const NIVEL: f32 = 34.0;
const SOBRE_BARRA: f32 = 10.0;
const BARRA: f32 = 8.0;
/// La marca del tope sobresale 3 px por arriba y por abajo de la barra.
const MARCA_TOPE: f32 = BARRA + 6.0;
const SOBRE_ESTADO: f32 = 10.0;
const ESTADO: f32 = 16.0;
/// El rótulo «Modo de energía» con su aire.
const SOBRE_SECCION: f32 = 16.0;
const SECCION: f32 = 16.0;
const BAJO_SECCION: f32 = 8.0;
/// Alto de cada pieza de la lista: la fila del lateral de Ajustes, con título
/// y subtítulo.
const FILA: f32 = 58.0;
/// Hueco entre piezas. Tan fino que las tres se leen como un bloque, pero cada
/// una sigue pareciendo pulsable por separado.
const HUECO_FILA: f32 = 3.0;
const PUNTO: f32 = 30.0;
const ICONO_PUNTO: f32 = 17.0;
/// Lado de la marca del perfil elegido y de la flecha del pie.
const MARCA: f32 = 16.0;
const SOBRE_PIE: f32 = 12.0;
const PIE: f32 = 44.0;

/// Un porcentaje como se escribe en cada idioma: «80 %» y «80%».
// 0.1.0
fn por_ciento(n: u8) -> String {
    if *ES {
        format!("{n} %")
    } else {
        format!("{n}%")
    }
}

/// Una duración legible: «4 h 12 min», «45 min». No «4:12», que se confunde
/// con una hora.
// 0.1.0
fn duracion(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

// 0.1.0
fn peso(w: Weight) -> iced_core::Font {
    control::peso(w)
}

/// El icono teñido, o un hueco de su tamaño si falta.
// 0.1.0
fn icono_o_hueco(icono: Option<&Icono>, px: f32, color: Color) -> PanelElement<'_> {
    match icono {
        Some(i) => icono::ver_teñido(i, px, Some(color)),
        None => Space::new().width(Length::Fixed(px)).into(),
    }
}

/// Un perfil de energía tal como lo nombra `power-profiles-daemon`.
struct Perfil {
    /// La clave que entiende el daemon: `power-saver`, `balanced`…
    clave: String,
    etiqueta: &'static str,
    /// Lo que hace, en el subtítulo de la fila.
    detalle: &'static str,
    /// El color que lo identifica: el mismo que lleva el pictograma del panel
    /// cuando ese perfil está puesto. Es lo que ata las dos cosas. No va solo:
    /// el rótulo al lado es lo que lo distingue para quien no separa el
    /// amarillo del verde.
    color: Color,
    /// El pictograma que va dentro del círculo de color: el segundo canal,
    /// además del rótulo, para quien no distingue los colores.
    icono: Option<Icono>,
}

/// Los pictogramas de los perfiles, **macizos**: el teñido de iced sustituye
/// el relleno, y uno de solo trazo saldría hueco. Balanza y cohete son los
/// `24/solid` de Heroicons; la hoja no existe allí y es la silueta de la de
/// Lucide, sin el nervio.
// 0.1.0
fn icono_perfil(clave: &str) -> Option<Icono> {
    let d = match clave {
        "power-saver" => {
            "M11 20A7 7 0 0 1 9.8 6.1C15.5 5 17 4.48 19 2c1 2 2 4.18 2 8 0 5.5-4.78 10-10 10Z"
        }
        "balanced" => {
            "M12 2.25a.75.75 0 0 1 .75.75v.756a49.106 49.106 0 0 1 9.152 1 .75.75 0 0 1-.152 1.485h-1.918l2.474 10.124a.75.75 0 0 1-.375.84A6.723 6.723 0 0 1 18.75 18a6.723 6.723 0 0 1-3.181-.795.75.75 0 0 1-.375-.84l2.474-10.124H12.75v13.28c1.293.076 2.534.343 3.697.776a.75.75 0 0 1-.262 1.453h-8.37a.75.75 0 0 1-.262-1.453c1.162-.433 2.404-.7 3.697-.775V6.24H6.332l2.474 10.124a.75.75 0 0 1-.375.84A6.723 6.723 0 0 1 5.25 18a6.723 6.723 0 0 1-3.181-.795.75.75 0 0 1-.375-.84L4.168 6.241H2.25a.75.75 0 0 1-.152-1.485 49.105 49.105 0 0 1 9.152-1V3a.75.75 0 0 1 .75-.75Zm4.878 13.543 1.872-7.662 1.872 7.662h-3.744Zm-9.756 0L5.25 8.131l-1.872 7.662h3.744Z"
        }
        "performance" => {
            "M9.315 7.584C12.195 3.883 16.695 1.5 21.75 1.5a.75.75 0 0 1 .75.75c0 5.056-2.383 9.555-6.084 12.436A6.75 6.75 0 0 1 9.75 22.5a.75.75 0 0 1-.75-.75v-4.131A15.838 15.838 0 0 1 6.382 15H2.25a.75.75 0 0 1-.75-.75 6.75 6.75 0 0 1 7.815-6.666ZM15 6.75a2.25 2.25 0 1 0 0 4.5 2.25 2.25 0 0 0 0-4.5Z M5.26 17.242a.75.75 0 1 0-.897-1.203 5.243 5.243 0 0 0-2.05 5.022.75.75 0 0 0 .625.627 5.243 5.243 0 0 0 5.022-2.051.75.75 0 1 0-1.202-.897 3.744 3.744 0 0 1-3.008 1.51c0-1.23.592-2.323 1.51-3.008Z"
        }
        _ => return None,
    };
    Some(icono::desde_svg(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" clip-rule="evenodd" d="{d}"/></svg>"##
    )))
}

/// El rayo y el aviso de la pastilla de estado: `bolt` y
/// `exclamation-triangle` de Heroicons Outline, con el trazo a 2,5 como pide el
/// HIG para iconos de 14 px.
const RAYO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m3.75 13.5 10.5-11.25L12 10.5h8.25L9.75 21.75 12 13.5H3.75Z"/></svg>"##;
const AVISO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z"/></svg>"##;

/// Traduce la clave del daemon: rótulo, qué hace y color.
// 0.1.0
fn nombrar(clave: &str) -> (&'static str, &'static str, Color) {
    match clave {
        "power-saver" => (
            t("Ahorro de energía", "Power saver"),
            t(
                "Más autonomía, menos potencia",
                "Longer battery life, less power",
            ),
            tema::perfil_ahorro(),
        ),
        "balanced" => (
            t("Equilibrado", "Balanced"),
            t("Potencia según lo que hagas", "Power adapts to what you do"),
            tema::perfil_equilibrado(),
        ),
        "performance" => (
            t("Alto rendimiento", "Performance"),
            t(
                "Máxima potencia, más consumo",
                "Maximum power, higher usage",
            ),
            tema::perfil_rendimiento(),
        ),
        // Los hay con un cuarto perfil propio del fabricante; se enseña en gris
        // en vez de esconderlo.
        _ => (
            t("Otro perfil", "Other profile"),
            t("Definido por el fabricante", "Defined by the manufacturer"),
            tema::TEXTO2,
        ),
    }
}

/// Los perfiles que ofrece el daemon, en el orden en que los da: de menos a más
/// consumo.
// 0.1.0
fn perfiles() -> Vec<Perfil> {
    let salida = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "Profiles",
        ])
        .output()
        .ok();
    let Some(salida) = salida.filter(|s| s.status.success()) else {
        return Vec::new();
    };
    let texto = String::from_utf8_lossy(&salida.stdout);
    // La propiedad es un `aa{sv}` y `busctl` la imprime en una línea con las
    // cadenas entrecomilladas. En vez de escribir un analizador del formato de
    // salida —que cambia entre versiones—, se buscan los valores que siguen a
    // la clave "Profile", que es lo único que hace falta.
    let mut claves = Vec::new();
    let mut trozos = texto.split('"');
    while let Some(t) = trozos.next() {
        if t.trim_end().ends_with("Profile") && t.trim_end().len() == "Profile".len() {
            // "Profile" s "power-saver": el valor es la siguiente cadena
            // entrecomillada, saltándose el tipo `s` que va entre medias.
            if let Some(valor) = trozos.nth(1) {
                claves.push(valor.to_string());
            }
        }
    }
    claves
        .into_iter()
        .map(|clave| {
            let (etiqueta, detalle, color) = nombrar(&clave);
            Perfil {
                icono: icono_perfil(&clave),
                clave,
                etiqueta,
                detalle,
                color,
            }
        })
        .collect()
}

/// Qué perfil está puesto ahora.
// 0.1.0
pub(super) fn perfil_activo() -> Option<String> {
    let salida = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "ActiveProfile",
        ])
        .output()
        .ok()
        .filter(|s| s.status.success())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    // `s "power-saver"`
    texto.split('"').nth(1).map(str::to_string)
}

/// En qué está la batería, que es lo que decide la pastilla, el color de la
/// barra y el renglón de estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fase {
    /// Un sobremesa: no hay batería.
    SinBateria,
    /// Bajo mínimos y sin enchufe.
    Baja,
    Descargando,
    Cargando,
    /// Enchufado y sin cargar: llena, o parada en el tope.
    Enchufado,
}

impl Fase {
    // 0.1.0
    fn de(bateria: Option<Battery>) -> Self {
        match bateria {
            None => Fase::SinBateria,
            Some(b) if b.charging => Fase::Cargando,
            Some(b) if b.plugged => Fase::Enchufado,
            Some(b) if b.percent <= 15 => Fase::Baja,
            Some(_) => Fase::Descargando,
        }
    }
}

pub struct Energia {
    bateria: Option<Battery>,
    /// El tope de carga del portátil, si lo hay.
    umbral: Option<u8>,
    perfiles: Vec<Perfil>,
    activo: Option<String>,
    /// Sobre qué fila está el ratón, para el realce.
    señalado: tema::Realce,
    /// Cuánto de «elegido» lleva cada perfil, en el orden de `perfiles`: el
    /// fondo y la marca se funden de uno a otro en vez de saltar.
    // añadido en la 0.2.0
    seleccion: Vec<tema::Transicion>,
    /// El porcentaje que dibuja la barra, de 0 a 1. Sigue a la batería con un
    /// muelle: al enchufar o al refrescar, la barra se desliza.
    nivel: tema::Transicion,
    /// El color de la barra va de `tinte_desde` a `tinte_hasta` según `tinte`
    /// (0 a 1). Cambia al cambiar de perfil, de fase o de carga.
    tinte: tema::Transicion,
    tinte_desde: Color,
    tinte_hasta: Color,
    /// Se cargan una vez: son los mismos en cada repintado.
    marca: Option<Icono>,
    flecha: Option<Icono>,
    rayo: Icono,
    aviso: Icono,
}

impl Energia {
    // 0.1.0
    // modified 0.2.0 -- transiciones de perfil elegido, nivel y color de la barra
    pub fn new() -> Self {
        let perfiles = perfiles();
        let activo = perfil_activo();
        let seleccion = perfiles
            .iter()
            .map(|p| {
                let elegido = activo.as_deref() == Some(p.clave.as_str());
                tema::Transicion::nueva(elegido as u8 as f32, tema::M_RAPIDO)
            })
            .collect();
        // Con tiempo restante: es la tarjeta que lo enseña, y solo se lee
        // mientras está abierta.
        let bateria = Battery::read(true);
        let nivel = bateria.map_or(0.0, |b| b.percent.min(100) as f32 / 100.0);
        let mut e = Self {
            bateria,
            umbral: crate::state::charge_limit(),
            perfiles,
            activo,
            señalado: tema::Realce::nuevo(),
            seleccion,
            nivel: tema::Transicion::nueva(nivel, tema::M_SUAVE),
            tinte: tema::Transicion::nueva(1.0, tema::M_SUAVE),
            tinte_desde: tema::TEXTO2,
            tinte_hasta: tema::TEXTO2,
            marca: icono::propio("comprobado"),
            flecha: icono::propio("chevron-derecha"),
            rayo: icono::desde_svg(RAYO),
            aviso: icono::desde_svg(AVISO),
        };
        // Al abrir, el color ya es el que toca: nada que animar delante de quien
        // acaba de abrir la tarjeta.
        e.tinte_desde = e.color_barra();
        e.tinte_hasta = e.tinte_desde;
        e
    }

    /// El color al que tiende la barra: rojo bajo, verde enchufado, el del
    /// perfil si descarga.
    // añadido en la 0.2.0
    fn color_barra(&self) -> Color {
        match Fase::de(self.bateria) {
            Fase::Baja => tema::rojo(),
            Fase::Cargando | Fase::Enchufado => tema::verde(),
            _ => self.color_activo(),
        }
    }

    /// Si el color de destino ha cambiado, arranca el fundido desde el que se
    /// veía en ese instante.
    // añadido en la 0.2.0
    fn sincronizar_color(&mut self) {
        let destino = self.color_barra();
        if destino == self.tinte_hasta {
            return;
        }
        self.tinte_desde = self.color_barra_ahora();
        self.tinte_hasta = destino;
        self.tinte.fijar(0.0);
        self.tinte.ir_a(1.0);
    }

    // añadido en la 0.2.0
    fn color_barra_ahora(&self) -> Color {
        tema::mezclar(
            self.tinte_desde,
            self.tinte_hasta,
            self.tinte.valor().clamp(0.0, 1.0),
        )
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        let perfiles = if self.perfiles.is_empty() {
            0.0
        } else {
            SOBRE_SECCION + SECCION + BAJO_SECCION + self.alto_lista()
        };
        (
            ANCHO,
            MARGEN * 2.0 + self.alto_cabecera() + perfiles + SOBRE_PIE + PIE,
        )
    }

    /// La cabecera entera: título, nivel, barra si hay batería y estado.
    // 0.1.0
    fn alto_cabecera(&self) -> f32 {
        let barra = if self.bateria.is_some() {
            SOBRE_BARRA + BARRA
        } else {
            0.0
        };
        TITULO + BAJO_TITULO + NIVEL + barra + SOBRE_ESTADO + ESTADO
    }

    /// Lo que ocupan las piezas de la lista.
    // 0.1.0
    fn alto_lista(&self) -> f32 {
        let filas = self.perfiles.len() as f32;
        if filas == 0.0 {
            return 0.0;
        }
        filas * FILA + (filas - 1.0) * HUECO_FILA
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("bateria")
    }

    /// La `y` donde empieza la lista de perfiles, relativa a la emergente.
    // 0.1.0
    fn y_lista(&self) -> f32 {
        MARGEN + self.alto_cabecera() + SOBRE_SECCION + SECCION + BAJO_SECCION
    }

    /// Qué fila de perfil cae en ese punto.
    // 0.1.0
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if !(MARGEN..=ANCHO - MARGEN).contains(&x) {
            return None;
        }
        let rel = y - self.y_lista();
        if rel < 0.0 {
            return None;
        }
        let i = (rel / (FILA + HUECO_FILA)) as usize;
        // El hueco entre piezas no es de nadie: pulsarlo no debe cambiar el
        // perfil por accidente.
        if rel - i as f32 * (FILA + HUECO_FILA) > FILA {
            return None;
        }
        (i < self.perfiles.len()).then_some(i)
    }

    /// ¿Cae el punto en el pie? Incluye el margen de debajo: es el borde de la
    /// tarjeta y apuntar al botón desde abajo no debe fallar por 16 px.
    // 0.1.0
    fn en_pie(&self, x: f32, y: f32) -> bool {
        let alto = self.size().1;
        y >= alto - MARGEN - PIE && (MARGEN..=ANCHO - MARGEN).contains(&x)
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.señalado
            .señalar(punto.and_then(|(x, y)| self.fila_en(x, y)))
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalado.animando()
            || self.nivel.animando()
            || self.tinte.animando()
            || self.seleccion.iter().any(|t| t.animando())
    }

    // 0.1.0
    // modified 0.1.1 -- recoge busctl en otro hilo sin dejar zombis
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if self.en_pie(x, y) {
            // `bateria` es el id de la página en BookOS Settings (`pages` en su
            // main.js). Con Ajustes ya abierto, la segunda instancia lo manda
            // por su socket y la ventana viva navega a esa página; sin abrir,
            // lo lee `get_startup_page` al arrancar.
            return Some(Accion::Lanzar("bookos-settings --page bateria".into()));
        }
        let i = self.fila_en(x, y)?;
        let clave = self.perfiles[i].clave.clone();
        if self.activo.as_deref() == Some(clave.as_str()) {
            return None;
        }
        // Esperado en un hilo aparte: sin `wait` el `busctl` terminado se
        // queda de zombi colgando del compositor, y esperarlo aquí detendría
        // el bucle lo que tarde el daemon en contestar.
        if let Ok(mut hijo) = Command::new("busctl")
            .args([
                "--system",
                "set-property",
                "net.hadess.PowerProfiles",
                "/net/hadess/PowerProfiles",
                "net.hadess.PowerProfiles",
                "ActiveProfile",
                "s",
                &clave,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            std::thread::spawn(move || {
                let _ = hijo.wait();
            });
        }
        // Se da por puesto sin releer: la lectura tardaría lo que tarde el
        // daemon en aplicarlo y la fila se quedaría sin marcar hasta el
        // siguiente refresco. Si la llamada falla, el próximo `new` lo corrige.
        for (p, t) in self.perfiles.iter().zip(self.seleccion.iter_mut()) {
            t.ir_a((p.clave == clave) as u8 as f32);
        }
        self.activo = Some(clave);
        self.sincronizar_color();
        None
    }

    /// Relee la batería y el tope mientras la tarjeta está abierta, que es
    /// cuando se mira el tiempo: sin esto se quedaba con la foto del instante
    /// en que se abrió y el panel, debajo, seguía contando. El tope también,
    /// porque se cambia en Ajustes con la tarjeta a la vista.
    // 0.1.0
    pub fn refrescar(&mut self) -> bool {
        let fresca = Battery::read(true);
        let umbral = crate::state::charge_limit();
        if fresca == self.bateria && umbral == self.umbral {
            return false;
        }
        self.bateria = fresca;
        self.umbral = umbral;
        self.nivel
            .ir_a(fresca.map_or(0.0, |b| b.percent.min(100) as f32 / 100.0));
        self.sincronizar_color();
        true
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// El color del perfil que está puesto, con el que se rellena la barra
    /// mientras descarga: el mismo que lleva el pictograma del panel, para que
    /// las dos cosas se lean como una.
    // 0.1.0
    fn color_activo(&self) -> Color {
        self.activo
            .as_deref()
            .and_then(|a| self.perfiles.iter().find(|p| p.clave == a))
            .map_or(tema::TEXTO2, |p| p.color)
    }

    /// El tope solo cuenta enchufado: descargando no frena nada y dibujarlo
    /// sería ruido.
    // 0.1.0
    fn tope_visible(&self) -> Option<u8> {
        self.umbral
            .filter(|_| self.bateria.is_some_and(|b| b.plugged))
    }

    /// La pastilla de la derecha del título: rótulo, icono y su color.
    // 0.1.0
    fn pastilla(&self) -> Option<(&'static str, &Icono, Color)> {
        match Fase::de(self.bateria) {
            Fase::Descargando => None,
            Fase::Baja => Some((t("Batería baja", "Low battery"), &self.aviso, tema::rojo())),
            Fase::Cargando => Some((t("Cargando", "Charging"), &self.rayo, tema::verde())),
            Fase::Enchufado => Some((t("Enchufado", "Plugged in"), &self.rayo, tema::verde())),
            Fase::SinBateria => Some((t("Corriente", "AC power"), &self.rayo, tema::verde())),
        }
    }

    /// El renglón de debajo de la barra.
    // 0.1.0
    fn estado(&self) -> String {
        let Some(bat) = self.bateria else {
            return t(
                "Este equipo funciona solo con corriente",
                "This computer runs on AC power only",
            )
            .into();
        };
        match (Fase::de(self.bateria), bat.minutes) {
            (Fase::Descargando | Fase::Baja, Some(m)) => {
                if *ES {
                    format!("{} restantes", duracion(m))
                } else {
                    format!("{} left", duracion(m))
                }
            }
            (Fase::Descargando | Fase::Baja, None) => t("Usando la batería", "On battery").into(),
            // Con tope, la meta es el tope: «llena en» prometería un 100 % al
            // que la batería no va a llegar.
            (Fase::Cargando, Some(m)) => match (self.umbral, *ES) {
                (Some(u), true) => format!("Al {} en {}", por_ciento(u), duracion(m)),
                (Some(u), false) => format!("{} in {}", por_ciento(u), duracion(m)),
                (None, true) => format!("Llena en {}", duracion(m)),
                (None, false) => format!("Full in {}", duracion(m)),
            },
            (Fase::Cargando, None) => t("Cargando la batería", "Charging the battery").into(),
            // Parada en el tope: se dice por qué no sube, que si no parece un
            // fallo del cargador.
            (Fase::Enchufado, _) if self.umbral.is_some_and(|u| bat.percent + 1 >= u) => t(
                "En pausa para cuidar la batería",
                "Paused to protect the battery",
            )
            .into(),
            (Fase::Enchufado, _) if bat.percent >= 99 => {
                t("Batería cargada", "Fully charged").into()
            }
            (Fase::Enchufado, _) => t("Enchufado, sin cargar", "Plugged in, not charging").into(),
            (Fase::SinBateria, _) => unreachable!("con batería leída la fase no es SinBateria"),
        }
    }

    /// Título y pastilla, el porcentaje grande, la barra y el renglón.
    // 0.1.0
    fn cabecera(&self) -> PanelElement<'_> {
        let mut titulo = row![
            text(t("Batería", "Battery"))
                .size(12.0)
                .font(peso(Weight::Medium))
                .color(tema::TEXTO2),
            Space::new().width(Length::Fill),
        ]
        .height(Length::Fixed(TITULO))
        .align_y(Vertical::Center);
        if let Some((rotulo, icono, color)) = self.pastilla() {
            titulo = titulo.push(
                container(
                    row![
                        icono::ver_teñido(icono, 14.0, Some(color)),
                        text(rotulo)
                            .size(12.0)
                            .font(peso(Weight::Medium))
                            .color(tema::texto()),
                    ]
                    .spacing(4)
                    .align_y(Vertical::Center),
                )
                .height(Length::Fixed(TITULO))
                .center_y(Length::Fixed(TITULO))
                .padding([0, 8])
                .style(|_theme: &iced_widget::Theme| container::Style {
                    background: Some(tema::superficie_en_tarjeta().into()),
                    border: Border {
                        radius: tema::R_CHIP.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }

        let fase = Fase::de(self.bateria);
        let nivel = match self.bateria {
            Some(b) => por_ciento(b.percent),
            None => t("Sin batería", "No battery").into(),
        };
        // Bajo mínimos el número también se pone rojo: a 28 px el rojo pasa el
        // 3:1 que pide el texto grande, y es lo que tiene que verse de lejos.
        let color_nivel = if fase == Fase::Baja {
            tema::rojo()
        } else {
            tema::texto()
        };
        let mut cabecera = column![
            titulo,
            Space::new().height(Length::Fixed(BAJO_TITULO)),
            container(
                text(nivel)
                    .size(28.0)
                    .font(peso(Weight::Bold))
                    .color(color_nivel)
            )
            .height(Length::Fixed(NIVEL))
            .center_y(Length::Fixed(NIVEL)),
        ];
        if self.bateria.is_some() {
            cabecera = cabecera
                .push(Space::new().height(Length::Fixed(SOBRE_BARRA)))
                .push(self.barra());
        }

        let mut renglon = row![
            text(lista::recortar(&self.estado(), INTERIOR - 70.0, 13.0))
                .size(13.0)
                .color(tema::TEXTO2),
            Space::new().width(Length::Fill),
        ]
        .height(Length::Fixed(ESTADO))
        .align_y(Vertical::Center);
        if let Some(u) = self.tope_visible() {
            let tope = if *ES {
                format!("Tope {}", por_ciento(u))
            } else {
                format!("Limit {}", por_ciento(u))
            };
            renglon = renglon.push(text(tope).size(13.0).color(tema::TEXTO2));
        }
        cabecera
            .push(Space::new().height(Length::Fixed(SOBRE_ESTADO)))
            .push(renglon)
            .into()
    }

    /// La barra de nivel con la marca del tope encima, si toca.
    ///
    /// Va en un `stack` de la altura de la marca, con la barra centrada, y no
    /// en la altura de la barra: así la marca sobresale 3 px por cada lado sin
    /// que la cabecera tenga que reservar ese aire en otro sitio.
    // 0.1.0
    fn barra(&self) -> PanelElement<'_> {
        let color = self.color_barra_ahora();
        // Nunca más estrecho que alto: con menos, iced recorta el radio y
        // asoma una barrita recta por las curvas (ver `control::pildora`).
        let lleno = (INTERIOR * self.nivel.valor().clamp(0.0, 1.0)).max(BARRA);
        let pista = container(
            container(Space::new())
                .width(Length::Fixed(lleno))
                .height(Length::Fixed(BARRA))
                .style(move |_theme: &iced_widget::Theme| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(INTERIOR))
        .height(Length::Fixed(BARRA))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::control_apagado().into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let aire = (MARCA_TOPE - BARRA) / 2.0;
        let mut capas = stack![
            container(pista)
                .height(Length::Fixed(MARCA_TOPE))
                .padding([aire, 0.0])
        ];
        if let Some(u) = self.tope_visible() {
            let x = INTERIOR * u as f32 / 100.0 - 1.0;
            capas = capas.push(row![
                Space::new().width(Length::Fixed(x)),
                container(Space::new())
                    .width(Length::Fixed(2.0))
                    .height(Length::Fixed(MARCA_TOPE))
                    .style(|_theme: &iced_widget::Theme| container::Style {
                        background: Some(tema::texto().into()),
                        border: Border {
                            radius: 1.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            ]);
        }
        container(capas).height(Length::Fixed(MARCA_TOPE)).into()
    }

    /// Una pieza de la lista segmentada.
    // 0.1.0
    // modified 0.1.1 -- en oscuro el acento al 20 %, como control::seleccionada
    fn fila(&self, i: usize) -> PanelElement<'_> {
        let perfil = &self.perfiles[i];
        // 0 a 1; con el muelle se pasa un poco, y un color no admite eso.
        let elegido = self.seleccion[i].valor().clamp(0.0, 1.0);
        let señalado = self.señalado.intensidad(i);
        let base = tema::superficie_en_tarjeta();

        // El elegido, con el acento al 10 % como lo seleccionado en cualquier
        // lista del sistema —al 20 % en oscuro, por lo mismo que en
        // `control::seleccionada`—; el color del perfil se queda en el círculo.
        let mezcla = if tema::es_claro() { 0.10 } else { 0.20 };
        let fondo = tema::mezclar(
            tema::mezclar(base, tema::tinta(), 0.05 * señalado),
            tema::mezclar(base, tema::acento(), mezcla + 0.06 * señalado),
            elegido,
        );
        let color = tema::mezclar(tema::texto(), tema::acento(), elegido);
        // Esquinas de control por fuera del bloque y de chip donde se tocan
        // dos piezas: las tres forman una sola figura.
        let ultima = i + 1 == self.perfiles.len();
        let (arriba, abajo) = (
            if i == 0 {
                tema::R_CONTROL
            } else {
                tema::R_CHIP
            },
            if ultima {
                tema::R_CONTROL
            } else {
                tema::R_CHIP
            },
        );
        let radio = Radius {
            top_left: arriba,
            top_right: arriba,
            bottom_right: abajo,
            bottom_left: abajo,
        };

        let color_punto = perfil.color;
        let punto = container(icono_o_hueco(
            perfil.icono.as_ref(),
            ICONO_PUNTO,
            tema::tinta_sobre(color_punto),
        ))
        .width(Length::Fixed(PUNTO))
        .height(Length::Fixed(PUNTO))
        .center_x(Length::Fixed(PUNTO))
        .center_y(Length::Fixed(PUNTO))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(color_punto.into()),
            border: Border {
                radius: (PUNTO / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        // Lo que queda para el texto: la pieza menos el relleno, el círculo,
        // su hueco y la marca con el suyo.
        let ancho_texto = INTERIOR - 28.0 - PUNTO - 14.0 - MARCA - 14.0;
        let textos = column![
            text(lista::recortar(perfil.etiqueta, ancho_texto, 14.0))
                .size(14.0)
                .font(peso(Weight::Medium))
                .color(color),
            text(lista::recortar(perfil.detalle, ancho_texto, 11.0))
                .size(11.0)
                .color(tema::TEXTO2),
        ]
        .spacing(2);
        let mut contenido = row![
            punto,
            Space::new().width(Length::Fixed(14.0)),
            textos,
            Space::new().width(Length::Fill),
        ]
        .align_y(Vertical::Center);
        // La marca se enciende con el fundido; el hueco que deja el spacer
        // `Fill` hace que su ausencia no mueva el texto.
        if elegido > 0.01 {
            contenido = contenido.push(icono_o_hueco(
                self.marca.as_ref(),
                MARCA,
                tema::alfa(tema::acento(), elegido),
            ));
        }
        container(contenido)
            .width(Length::Fixed(INTERIOR))
            .height(Length::Fixed(FILA))
            .center_y(Length::Fixed(FILA))
            .padding([0, 14])
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: radio,
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// El pie: «Preferencias de batería», que abre su página en Ajustes. En el
    /// color del texto y no en gris: es un botón, y en gris parecía apagado.
    // 0.1.0
    fn pie(&self) -> PanelElement<'_> {
        control::grupo(
            container(
                row![
                    text(t("Preferencias de batería", "Battery settings"))
                        .size(14.0)
                        .font(peso(Weight::Medium))
                        .color(tema::texto()),
                    Space::new().width(Length::Fill),
                    icono_o_hueco(self.flecha.as_ref(), MARCA, tema::TEXTO2),
                ]
                .align_y(Vertical::Center),
            )
            .height(Length::Fixed(PIE))
            .center_y(Length::Fixed(PIE))
            .padding([0, 14])
            .into(),
            INTERIOR,
            0.0,
        )
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let mut contenido = column![self.cabecera()];
        if !self.perfiles.is_empty() {
            let mut filas = column![].spacing(HUECO_FILA);
            for i in 0..self.perfiles.len() {
                filas = filas.push(self.fila(i));
            }
            contenido = contenido
                .push(Space::new().height(Length::Fixed(SOBRE_SECCION)))
                .push(
                    container(
                        text(t("Modo de energía", "Power mode"))
                            .size(12.0)
                            .font(peso(Weight::Medium))
                            .color(tema::TEXTO2),
                    )
                    .height(Length::Fixed(SECCION))
                    .center_y(Length::Fixed(SECCION)),
                )
                .push(Space::new().height(Length::Fixed(BAJO_SECCION)))
                .push(filas);
        }
        contenido = contenido
            .push(Space::new().height(Length::Fixed(SOBRE_PIE)))
            .push(self.pie());
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn con_perfiles(claves: &[&str]) -> Energia {
        let mut e = Energia::new();
        e.perfiles = claves
            .iter()
            .map(|c| Perfil {
                clave: (*c).into(),
                etiqueta: "X",
                detalle: "Y",
                color: tema::TEXTO2,
                icono: None,
            })
            .collect();
        e
    }

    /// El hueco entre piezas no pertenece a ninguna: pulsarlo no debe cambiar
    /// el perfil.
    #[test]
    fn el_hueco_entre_filas_no_es_de_nadie() {
        let e = con_perfiles(&["a", "b"]);
        let y0 = e.y_lista();
        assert_eq!(e.fila_en(50.0, y0 + 1.0), Some(0));
        assert_eq!(e.fila_en(50.0, y0 + FILA + HUECO_FILA / 2.0), None);
        assert_eq!(e.fila_en(50.0, y0 + FILA + HUECO_FILA + 1.0), Some(1));
        assert_eq!(e.fila_en(50.0, y0 - 1.0), None, "por encima de la lista");
    }

    /// Sin perfiles —un equipo sin `power-profiles-daemon`— la tarjeta encoge
    /// en vez de dejar un hueco vacío debajo.
    #[test]
    fn sin_perfiles_la_tarjeta_encoge() {
        let mut e = con_perfiles(&["a"]);
        let con = e.size().1;
        e.perfiles.clear();
        assert!(
            e.size().1 < con,
            "{} debería ser menor que {con}",
            e.size().1
        );
    }

    /// El pie abre la página de batería de Ajustes, con perfiles y sin ellos,
    /// y también pulsando en el margen de debajo.
    #[test]
    fn el_pie_abre_la_pagina_de_bateria() {
        for claves in [&["a", "b", "c"][..], &[]] {
            let mut e = con_perfiles(claves);
            let alto = e.size().1;
            for y in [alto - MARGEN - PIE / 2.0, alto - 2.0] {
                let accion = e.pulsar(ANCHO / 2.0, y);
                assert!(
                    matches!(&accion, Some(Accion::Lanzar(cmd)) if cmd == "bookos-settings --page bateria"),
                    "y={y} con {} perfiles: {accion:?}",
                    claves.len()
                );
            }
        }
    }

    /// El pie y la última fila no se pisan: pulsar la última fila cambia el
    /// perfil y no abre Ajustes.
    #[test]
    fn la_ultima_fila_no_es_el_pie() {
        let mut e = con_perfiles(&["a", "b", "c"]);
        e.activo = Some("a".into());
        let y = e.y_lista() + 2.0 * (FILA + HUECO_FILA) + FILA - 1.0;
        assert_eq!(e.fila_en(ANCHO / 2.0, y), Some(2));
        assert!(!e.en_pie(ANCHO / 2.0, y));
    }

    #[test]
    fn duraciones_legibles() {
        assert_eq!(duracion(45), "45 min");
        assert_eq!(duracion(60), "1 h");
        assert_eq!(duracion(252), "4 h 12 min");
    }

    /// Con un sobremesa no hay barra y la cabecera es más baja.
    #[test]
    fn sin_bateria_no_hay_barra() {
        let mut e = con_perfiles(&["a"]);
        e.bateria = Some(Battery {
            percent: 50,
            charging: false,
            plugged: false,
            minutes: None,
        });
        let con = e.alto_cabecera();
        e.bateria = None;
        assert_eq!(e.alto_cabecera(), con - SOBRE_BARRA - BARRA);
        assert_eq!(Fase::de(None), Fase::SinBateria);
    }
}
