//! Los medidores de texto del panel: el monitor del sistema, el tráfico de red
//! y el consumo de la batería.
//!
//! No hay tarjeta detrás: son para mirar, no para pulsar. Por eso comparten
//! una sola estructura —lo único que cambia es qué leen y cómo lo escriben— en
//! vez de un módulo por widget.
//!
//! **Despiertan cada 2 s.** El reloj despierta una vez por minuto a propósito
//! (ver `reloj.rs`), y estos no pueden: un porcentaje de CPU que se actualiza
//! cada minuto no dice nada. El coste solo se paga si el usuario los pone en el
//! panel; con ellos fuera de `panel.conf` no existen y el panel despierta como
//! antes.

use std::time::{Duration, Instant};

use iced_widget::text;

use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::{Widget, ancho_de};

/// Cada cuánto vuelven a leer. Un intervalo menor hace bailar los dígitos sin
/// aportar información, y uno mayor deja el porcentaje de CPU obsoleto.
const INTERVALO: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tipo {
    Monitor,
    Trafico,
    Consumo,
}

pub struct Medidor {
    tipo: Tipo,
    texto: String,
    /// (total, ocioso) de `/proc/stat` en la lectura anterior: el uso de la CPU
    /// es la diferencia entre dos lecturas, no un valor que se pueda leer.
    cpu: Option<(u64, u64)>,
    /// Bytes recibidos y enviados, y cuándo se leyeron, por la misma razón.
    red: Option<(u64, u64, Instant)>,
}

impl Medidor {
    // añadido en la 0.2.0
    pub fn monitor() -> Self {
        Self::nuevo(Tipo::Monitor)
    }

    // añadido en la 0.2.0
    pub fn trafico() -> Self {
        Self::nuevo(Tipo::Trafico)
    }

    // añadido en la 0.2.0
    pub fn consumo() -> Self {
        Self::nuevo(Tipo::Consumo)
    }

    fn nuevo(tipo: Tipo) -> Self {
        let mut medidor = Self {
            tipo,
            texto: String::new(),
            cpu: None,
            red: None,
        };
        medidor.refrescar();
        medidor
    }

    fn leer(&mut self) -> String {
        match self.tipo {
            Tipo::Monitor => self.leer_monitor(),
            Tipo::Trafico => self.leer_trafico(),
            Tipo::Consumo => leer_consumo().map_or_else(String::new, |w| format!("{} W", coma(w))),
        }
    }

    fn leer_monitor(&mut self) -> String {
        let ahora = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|t| parsear_cpu(&t));
        // La primera lectura no tiene con qué compararse: se enseña 0 % y no
        // un porcentaje inventado desde el arranque de la máquina.
        let uso = match (self.cpu, ahora) {
            (Some(antes), Some(ahora)) => uso_cpu(antes, ahora),
            _ => 0,
        };
        self.cpu = ahora;
        let mut partes = vec![format!("CPU {uso}%")];
        if let Some(ram) = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|t| parsear_memoria(&t))
        {
            partes.push(format!("RAM {ram}%"));
        }
        if let Some(grados) = temperatura() {
            partes.push(format!("{grados}°"));
        }
        partes.join(" · ")
    }

    fn leer_trafico(&mut self) -> String {
        let Some((rx, tx)) = std::fs::read_to_string("/proc/net/dev")
            .ok()
            .and_then(|t| parsear_red(&t))
        else {
            return String::new();
        };
        let ahora = Instant::now();
        let texto = match self.red {
            Some((rx0, tx0, desde)) => {
                let segundos = ahora.duration_since(desde).as_secs_f64().max(0.001);
                // Un contador que baja —se reinició una interfaz— es 0, no un
                // desbordamiento que dé exabytes por segundo.
                let bajada = rx.saturating_sub(rx0) as f64 / segundos;
                let subida = tx.saturating_sub(tx0) as f64 / segundos;
                format!("↓ {}  ↑ {}", velocidad(bajada), velocidad(subida))
            }
            None => format!("↓ {}  ↑ {}", velocidad(0.0), velocidad(0.0)),
        };
        self.red = Some((rx, tx, ahora));
        texto
    }
}

impl Widget for Medidor {
    // añadido en la 0.2.0
    fn nombre(&self) -> &'static str {
        match self.tipo {
            Tipo::Monitor => "monitor",
            Tipo::Trafico => "trafico",
            Tipo::Consumo => "consumo",
        }
    }

    // añadido en la 0.2.0
    fn refrescar(&mut self) -> bool {
        let texto = self.leer();
        if texto == self.texto {
            return false;
        }
        self.texto = texto;
        true
    }

    // añadido en la 0.2.0
    fn proxima_alarma(&self) -> Option<Duration> {
        Some(INTERVALO)
    }

    // añadido en la 0.2.0
    fn ancho(&self) -> f32 {
        ancho_de(&self.texto, tema::T_CUERPO)
    }

    // añadido en la 0.2.0
    fn ver(&self) -> PanelElement<'_> {
        text(self.texto.clone())
            .size(tema::T_CUERPO)
            .color(TEXT())
            .into()
    }
}

/// Un decimal con coma, que es como se escribe en castellano.
fn coma(valor: f64) -> String {
    format!("{valor:.1}").replace('.', ",")
}

/// De bytes por segundo a algo que se lea de un vistazo. El ancho del texto
/// salta entre `999 KB/s` y `1,0 MB/s`; es inevitable y el panel lo absorbe.
fn velocidad(bytes: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    if bytes >= MB {
        format!("{} MB/s", coma(bytes / MB))
    } else if bytes >= KB {
        format!("{:.0} KB/s", bytes / KB)
    } else {
        format!("{bytes:.0} B/s")
    }
}

/// `(total, ocioso)` de la línea `cpu` de `/proc/stat`.
///
/// Se suman los ocho primeros contadores —user, nice, system, idle, iowait,
/// irq, softirq, steal—: `guest` ya va dentro de `user`, y contarlo otra vez
/// inflaría el total. Esperar a un disco (`iowait`) cuenta como ocioso: la CPU
/// no está haciendo nada.
fn parsear_cpu(texto: &str) -> Option<(u64, u64)> {
    let linea = texto.lines().next()?.strip_prefix("cpu")?;
    let campos: Vec<u64> = linea
        .split_whitespace()
        .take(8)
        .map(|c| c.parse().ok())
        .collect::<Option<_>>()?;
    let [_, _, _, ocioso, espera, ..] = campos[..] else {
        return None;
    };
    Some((campos.iter().sum(), ocioso + espera))
}

fn uso_cpu((total0, ocioso0): (u64, u64), (total1, ocioso1): (u64, u64)) -> u64 {
    let total = total1.saturating_sub(total0);
    if total == 0 {
        return 0;
    }
    let ocioso = ocioso1.saturating_sub(ocioso0).min(total);
    (total - ocioso) * 100 / total
}

/// El porcentaje de memoria en uso, contando como libre lo que el kernel puede
/// devolver (`MemAvailable`, que incluye la caché) y no solo `MemFree`: con
/// `MemFree` un sistema sano siempre parece a punto de agotarse.
fn parsear_memoria(texto: &str) -> Option<u64> {
    let kb = |clave: &str| -> Option<u64> {
        texto
            .lines()
            .find_map(|l| l.strip_prefix(clave)?.strip_prefix(':'))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let total = kb("MemTotal")?;
    let disponible = kb("MemAvailable")?;
    (total > 0).then(|| total.saturating_sub(disponible) * 100 / total)
}

/// Los bytes recibidos y enviados por todas las interfaces menos `lo`: el
/// tráfico de la máquina consigo misma no es tráfico de red.
fn parsear_red(texto: &str) -> Option<(u64, u64)> {
    let mut hay = false;
    let (mut rx, mut tx) = (0u64, 0u64);
    for linea in texto.lines().skip(2) {
        let (interfaz, datos) = linea.split_once(':')?;
        if interfaz.trim() == "lo" {
            continue;
        }
        let campos: Vec<&str> = datos.split_whitespace().collect();
        // Los de recepción son los ocho primeros; el de bytes enviados es el
        // primero de los de transmisión.
        let (Some(r), Some(t)) = (campos.first(), campos.get(8)) else {
            continue;
        };
        rx += r.parse::<u64>().ok()?;
        tx += t.parse::<u64>().ok()?;
        hay = true;
    }
    hay.then_some((rx, tx))
}

/// La temperatura que se enseña, en grados.
///
/// Se prefiere la del paquete del procesador. Quedarse con la máxima de todas
/// las zonas daría la de la tarjeta wifi, que en este equipo va 5 ° por encima
/// y no es lo que alguien quiere decir con «temperatura».
fn temperatura() -> Option<i64> {
    let zonas: Vec<(String, i64)> = std::fs::read_dir("/sys/class/thermal")
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("thermal_zone"))
        .filter_map(|e| {
            let tipo = std::fs::read_to_string(e.path().join("type")).ok()?;
            let milis = std::fs::read_to_string(e.path().join("temp")).ok()?;
            Some((tipo.trim().to_owned(), milis.trim().parse().ok()?))
        })
        .collect();
    elegir_temperatura(&zonas)
}

fn elegir_temperatura(zonas: &[(String, i64)]) -> Option<i64> {
    let milis = zonas
        .iter()
        .find(|(tipo, _)| tipo == "x86_pkg_temp")
        .or_else(|| zonas.iter().max_by_key(|(_, m)| *m))?
        .1;
    // Un sensor que no responde da 0 o valores negativos: no es una lectura.
    (milis > 0).then_some(milis / 1000)
}

/// Los vatios que gasta la batería ahora mismo. Con `power_now` si el
/// controlador lo da; si no, corriente por tensión, que es lo que da este
/// equipo.
fn leer_consumo() -> Option<f64> {
    let bateria = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .find(|e| {
            std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "Battery")
        })?
        .path();
    let numero = |archivo: &str| -> Option<f64> {
        std::fs::read_to_string(bateria.join(archivo))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    // Las dos magnitudes están en micro-unidades: µW, o µA por µV.
    let vatios = numero("power_now")
        .map(|uw| uw / 1e6)
        .or_else(|| Some(numero("current_now")? * numero("voltage_now")? / 1e12))?;
    // Algunos controladores dan la corriente con signo según el sentido.
    Some(vatios.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_uso_de_cpu_es_la_diferencia_entre_dos_lecturas() {
        let antes = parsear_cpu("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 1 1 1").unwrap();
        let ahora = parsear_cpu("cpu  200 0 200 1000 0 0 0 0 0 0").unwrap();
        // Entre las dos lecturas pasan 400 unidades y 200 son de CPU ociosa.
        assert_eq!(uso_cpu(antes, ahora), 50);
    }

    /// `guest` ya va dentro de `user`: sumarlo otra vez daría un total mayor y
    /// un uso menor que el real.
    #[test]
    fn el_total_no_cuenta_guest() {
        let (total, ocioso) = parsear_cpu("cpu  10 20 30 40 50 60 70 80 999 999").unwrap();
        assert_eq!(total, 360);
        assert_eq!(ocioso, 90);
    }

    #[test]
    fn sin_diferencia_el_uso_es_cero() {
        assert_eq!(uso_cpu((100, 50), (100, 50)), 0);
    }

    #[test]
    fn la_memoria_usa_disponible_y_no_libre() {
        let texto = "MemTotal:       1000 kB\nMemFree:          10 kB\nMemAvailable:    600 kB\n";
        assert_eq!(parsear_memoria(texto), Some(40));
    }

    #[test]
    fn el_trafico_ignora_el_loopback() {
        let texto = "Inter-|   Receive\n face |bytes\n    lo: 500 1 0 0 0 0 0 0 500 1 0 0 0 0 0 0\n wlo1: 1000 1 0 0 0 0 0 0 200 1 0 0 0 0 0 0\n";
        assert_eq!(parsear_red(texto), Some((1000, 200)));
    }

    #[test]
    fn sin_interfaces_no_hay_trafico() {
        let texto =
            "Inter-|   Receive\n face |bytes\n    lo: 500 1 0 0 0 0 0 0 500 1 0 0 0 0 0 0\n";
        assert_eq!(parsear_red(texto), None);
    }

    #[test]
    fn prefiere_el_paquete_del_procesador() {
        let zonas = vec![
            ("iwlwifi_1".to_owned(), 43_000),
            ("x86_pkg_temp".to_owned(), 38_000),
        ];
        assert_eq!(elegir_temperatura(&zonas), Some(38));
        let sin_paquete = vec![("a".to_owned(), 30_000), ("b".to_owned(), 41_000)];
        assert_eq!(elegir_temperatura(&sin_paquete), Some(41));
        assert_eq!(elegir_temperatura(&[("a".to_owned(), 0)]), None);
    }

    #[test]
    fn las_velocidades_se_escriben_con_su_unidad() {
        assert_eq!(velocidad(0.0), "0 B/s");
        assert_eq!(velocidad(2048.0), "2 KB/s");
        assert_eq!(velocidad(1_572_864.0), "1,5 MB/s");
    }
}
