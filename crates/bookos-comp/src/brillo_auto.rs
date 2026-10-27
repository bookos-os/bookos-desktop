//! Brillo automático: la pantalla y la luz del teclado siguen al sensor de luz
//! ambiente.
//!
//! Los lux salen de iio-sensor-proxy y no de sysfs. El sensor HID del ISH
//! empuja lecturas por su buffer cuando la luz cambia, pero ese buffer
//! (`/dev/iio:device0`) es de root y ya lo tiene abierto iio-sensor-proxy. Leer
//! `in_illuminance_raw` desde aquí obligaría a sondear, y cada lectura es una
//! petición HID al ISH. El proxy manda `PropertiesChanged` solo cuando la luz
//! cambia: entre cambio y cambio no se despierta nada.
//!
//! Como las notificaciones, el bus vive en su propio hilo y los lux llegan al
//! bucle por un canal de calloop. El sensor solo se reclama (`ClaimLight`)
//! mientras haya algo en automático; sin reclamar, el proxy lo deja parado.
//!
//! ## Cambios a mano
//!
//! No se comprueba de dónde viene un cambio (la tecla Fn, la tarjeta, `busctl`
//! o el firmware): antes de ajustar se relee sysfs y se compara con lo último
//! que puso el automático.
//! - **Pantalla**: la diferencia se queda como desvío sobre la curva. Quien la
//!   sube un 20 % en un cuarto oscuro la tiene un 20 % más alta también con
//!   luz, en vez de verla volver a la curva con el siguiente cambio de luz.
//! - **Teclado**: solo se toca al cruzar de oscuro a claro o al revés. Lo que
//!   se ponga a mano dura hasta el siguiente cruce, y el último nivel encendido
//!   a mano es el que se usa al volver a encender.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};

use crate::state::BookosComp;

/// Tras escribir un brillo, sysfs tarda en reflejarlo: la escritura va por
/// `busctl` en el hilo de `retroiluminacion`. Durante este margen, una lectura
/// distinta se achaca a la escritura en curso y no a la mano del usuario.
const MARGEN_ESCRITURA: Duration = Duration::from_secs(2);

/// Lecturas en lux para encender y apagar el teclado. Separados para que una
/// luz justo en el límite no lo encienda y lo apague sin parar. No salen de
/// ninguna medición salvo una: el cuarto de pruebas con luz tenue daba 18 lux.
const TECLADO_ENCENDER: f64 = 15.0;
const TECLADO_APAGAR: f64 = 50.0;

const PASO_RAMPA: Duration = Duration::from_millis(16);
// Al oscurecer hay tiempo para adaptar la vista; al salir a la luz hace falta
// recuperar antes la legibilidad. La rampa sustituye la espera de tres segundos.
const DURACION_BAJADA: Duration = Duration::from_millis(900);
const DURACION_SUBIDA: Duration = Duration::from_millis(500);

struct Transicion {
    desde: f64,
    objetivo: u8,
    inicio: Instant,
    duracion: Duration,
}

impl Transicion {
    fn nueva(desde: f64, objetivo: u8, ahora: Instant) -> Self {
        Self {
            desde,
            objetivo,
            inicio: ahora,
            duracion: if bookos_shell::tema::efectos_reducidos() {
                Duration::ZERO
            } else if f64::from(objetivo) < desde {
                DURACION_BAJADA
            } else {
                DURACION_SUBIDA
            },
        }
    }

    fn terminada(&self, ahora: Instant) -> bool {
        ahora.duration_since(self.inicio) >= self.duracion
    }

    fn nivel(&self, ahora: Instant) -> f64 {
        if self.terminada(ahora) {
            return f64::from(self.objetivo);
        }
        let t = ahora.duration_since(self.inicio).as_secs_f64() / self.duracion.as_secs_f64();
        // Se conserva la fracción hasta escribir en la escala del panel: con
        // porcentajes enteros, un cambio pequeño solo tenía dos o tres saltos.
        let avance = t * t * (3.0 - 2.0 * t);
        self.desde + (f64::from(self.objetivo) - self.desde) * avance
    }

    fn dirigir(&mut self, objetivo: u8, ahora: Instant) {
        if self.objetivo != objetivo {
            *self = Self::nueva(self.nivel(ahora), objetivo, ahora);
        }
    }
}

struct Rampa {
    token: RegistrationToken,
    transicion: Transicion,
}

/// Lo que se pide al hilo del bus.
enum Orden {
    Reclamar,
    Soltar,
}

pub struct Estado {
    elegido: bookos_shell::BrilloAutomatico,
    ordenes: mpsc::Sender<Orden>,
    reclamado: bool,
    lux: Option<f64>,
    /// Último nivel de pantalla que puso el automático.
    pantalla_puesta: Option<u8>,
    /// Lo que el usuario movió la pantalla respecto a la curva, en puntos.
    desvio: i32,
    /// `Some(true)` con el teclado en la zona oscura, `None` antes de decidir.
    teclado_oscuro: Option<bool>,
    /// Nivel al que se enciende el teclado. Empieza en 1 y se actualiza con el
    /// que el usuario tenga encendido.
    teclado_encendido: u32,
    escrito: Option<Instant>,
    /// Último nivel pedido con la tecla. Mientras sysfs no lo refleje, la
    /// siguiente pulsación parte de aquí y no de la lectura atrasada.
    a_mano: Option<u8>,
    /// Último nivel de teclado pedido. Sus LEDs también tardan en reflejar la
    /// escritura asíncrona, pero tienen escala propia y no comparten el nivel.
    teclado_a_mano: Option<(String, u32, Instant)>,
    rampa: Option<Rampa>,
}

impl Estado {
    pub fn arrancar(
        loop_handle: &LoopHandle<'static, BookosComp>,
        elegido: bookos_shell::BrilloAutomatico,
    ) -> anyhow::Result<Self> {
        let (lux_tx, lux_rx) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(lux_rx, |evento, _, state| {
                if let smithay::reexports::calloop::channel::Event::Msg(lux) = evento {
                    recibir(state, lux);
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(luz): {err}"))?;
        let (ordenes, ordenes_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("bookos-luz".into())
            .spawn(move || escuchar(lux_tx, ordenes_rx))?;
        bookos_shell::retroiluminacion::aplicar_automatico(elegido);
        let mut estado = Self {
            elegido,
            ordenes,
            reclamado: false,
            lux: None,
            pantalla_puesta: None,
            desvio: 0,
            teclado_oscuro: None,
            teclado_encendido: 1,
            escrito: None,
            a_mano: None,
            teclado_a_mano: None,
            rampa: None,
        };
        estado.reclamar_si_hace_falta();
        Ok(estado)
    }

    fn reclamar_si_hace_falta(&mut self) {
        let quiere = self.elegido.pantalla || self.elegido.teclado;
        if quiere == self.reclamado {
            return;
        }
        self.reclamado = quiere;
        let orden = if quiere {
            Orden::Reclamar
        } else {
            Orden::Soltar
        };
        let _ = self.ordenes.send(orden);
    }
}

/// El hilo del bus: reclama o suelta el sensor según las órdenes y reenvía cada
/// lectura en lux.
fn escuchar(
    lux: smithay::reexports::calloop::channel::Sender<f64>,
    ordenes: mpsc::Receiver<Orden>,
) {
    let proxy = zbus::blocking::Connection::system().and_then(|conexion| {
        zbus::blocking::Proxy::new(
            &conexion,
            "net.hadess.SensorProxy",
            "/net/hadess/SensorProxy",
            "net.hadess.SensorProxy",
        )
    });
    let proxy = match proxy {
        Ok(proxy) => proxy,
        Err(err) => {
            tracing::warn!("sin iio-sensor-proxy, no habrá brillo automático: {err}");
            return;
        }
    };
    let escucha = proxy.clone();
    let lanzado = std::thread::Builder::new()
        .name("bookos-luz-lux".into())
        .spawn(move || {
            for cambio in escucha.receive_property_changed::<f64>("LightLevel") {
                // Con otra unidad («vendor») los números no son lux y la curva
                // no significa nada: mejor no mover el brillo que moverlo mal.
                let unidad = escucha.cached_property::<String>("LightLevelUnit");
                if !matches!(unidad, Ok(Some(ref u)) if u == "lux") {
                    continue;
                }
                if let Ok(valor) = cambio.get()
                    && lux.send(valor).is_err()
                {
                    return;
                }
            }
        });
    if let Err(err) = lanzado {
        tracing::warn!("no se pudo escuchar el sensor de luz: {err}");
        return;
    }
    for orden in ordenes {
        let (metodo, texto) = match orden {
            Orden::Reclamar => ("ClaimLight", "reclamar"),
            Orden::Soltar => ("ReleaseLight", "soltar"),
        };
        if let Err(err) = proxy.call_method(metodo, &()) {
            tracing::warn!("no se pudo {texto} el sensor de luz: {err}");
        }
    }
}

/// Lo que eligió el usuario desde la tarjeta o desde Settings.
pub fn elegir(state: &mut BookosComp, elegido: bookos_shell::BrilloAutomatico) {
    let e = &mut state.brillo_auto;
    bookos_shell::retroiluminacion::aplicar_automatico(elegido);
    // Al encender uno, se empieza de cero con él: un desvío o una zona de hace
    // una hora no dicen nada de lo que el usuario quiere ahora.
    if elegido.pantalla && !e.elegido.pantalla {
        e.pantalla_puesta = None;
        e.desvio = 0;
    }
    if elegido.teclado && !e.elegido.teclado {
        e.teclado_oscuro = None;
    }
    e.elegido = elegido;
    e.reclamar_si_hace_falta();
    if !elegido.pantalla {
        bookos_shell::retroiluminacion::poner_objetivo_sensor(None);
    }
    if !elegido.pantalla
        && let Some(rampa) = e.rampa.take()
    {
        state.loop_handle.remove(rampa.token);
    }
    evaluar(state);
    // El sol del panel lleva la «A» mientras la pantalla va sola: cambiar la
    // preferencia no toca el brillo, así que ningún evento de udev lo avisaría.
    if let Some(shell) = state.shell.as_mut() {
        shell.refresh();
    }
}

/// Nivel del que parte la tecla de brillo.
///
/// Con la tecla mantenida llegan 25 pulsaciones por segundo y cada escritura
/// tarda en verse en sysfs (va por `busctl`): partir de la lectura repetía el
/// mismo nivel varias veces y luego saltaba, y la pantalla seguía moviéndose
/// después de soltar.
pub fn partida_tecla(state: &BookosComp) -> Option<u8> {
    let e = &state.brillo_auto;
    match (e.a_mano, e.escrito) {
        (Some(nivel), Some(t)) if t.elapsed() < MARGEN_ESCRITURA => Some(nivel),
        _ => bookos_shell::brillo_actual(),
    }
}

/// La tecla ha pedido `nivel`. El automático lo aprende como desvío en el acto
/// y suelta su rampa: si no, la rampa en curso seguía escribiendo su objetivo
/// encima del de la tecla y el brillo bailaba al soltar.
pub fn tecla(state: &mut BookosComp, nivel: u8) {
    crate::atenuacion::olvidar(state);
    let e = &mut state.brillo_auto;
    if let Some(rampa) = e.rampa.take() {
        state.loop_handle.remove(rampa.token);
    }
    if let Some(puesta) = e.pantalla_puesta {
        e.desvio = (e.desvio + i32::from(nivel) - i32::from(puesta)).clamp(-60, 60);
        e.pantalla_puesta = Some(nivel);
    }
    e.a_mano = Some(nivel);
    e.escrito = Some(Instant::now());
}

pub fn partida_tecla_teclado(state: &BookosComp, dispositivo: &str, actual: u32) -> u32 {
    state
        .brillo_auto
        .teclado_a_mano
        .as_ref()
        .filter(|(d, _, t)| d == dispositivo && t.elapsed() < MARGEN_ESCRITURA)
        .map_or(actual, |(_, nivel, _)| *nivel)
}

pub fn tecla_teclado(state: &mut BookosComp, dispositivo: &str, nivel: u32) {
    state.brillo_auto.teclado_a_mano = Some((dispositivo.to_owned(), nivel, Instant::now()));
    state.brillo_auto.escrito = Some(Instant::now());
}

fn recibir(state: &mut BookosComp, lux: f64) {
    if !lux.is_finite() {
        return;
    }
    state.brillo_auto.lux = Some(lux);
    evaluar(state);
}

/// El compositor ha puesto `nivel` por su cuenta (la atenuación). A diferencia
/// de [`tecla`] no cuenta como desvío del usuario: el automático tiene que dar
/// por bueno ese nivel y no «corregirlo» al volver, ni aprender de él.
pub fn fijar(state: &mut BookosComp, nivel: u8) {
    let e = &mut state.brillo_auto;
    if let Some(rampa) = e.rampa.take() {
        state.loop_handle.remove(rampa.token);
    }
    if e.pantalla_puesta.is_some() {
        e.pantalla_puesta = Some(nivel);
    }
    e.a_mano = Some(nivel);
    e.escrito = Some(Instant::now());
}

/// Ajusta lo que esté en automático a la última lectura.
///
/// Se llama también al volver de DPMS: con la pantalla apagada no se ajusta
/// nada, y la luz pudo cambiar entretanto.
pub fn evaluar(state: &mut BookosComp) {
    let Some(lux) = state.brillo_auto.lux else {
        return;
    };
    // Atenuada, la pantalla está donde el compositor la ha puesto: ajustarla a
    // la luz la devolvería a su brillo en mitad de la advertencia.
    if !state.dpms_encendido || state.atenuacion.activa() {
        return;
    }
    if state.brillo_auto.elegido.pantalla {
        pantalla(state, lux);
    }
    if state.brillo_auto.elegido.teclado {
        teclado(&mut state.brillo_auto, lux);
    }
}

/// El brillo que toca con esa luz, en tanto por ciento.
///
/// Logarítmica porque así se percibe la luz: de 1 a 10 lux se nota tanto como
/// de 100 a 1000. Da 12 % a oscuras, 40 % a 18 lux, 78 % a 1000 y 100 % desde
/// unos 10 000, que es luz de día cerca de una ventana.
fn curva(lux: f64) -> i32 {
    (12.0 + 22.0 * (1.0 + lux.max(0.0)).log10()).round() as i32
}

fn reciente(e: &Estado) -> bool {
    e.rampa.is_some() || e.escrito.is_some_and(|t| t.elapsed() < MARGEN_ESCRITURA)
}

fn pantalla(state: &mut BookosComp, lux: f64) {
    bookos_shell::retroiluminacion::poner_objetivo_sensor(Some(curva(lux).clamp(5, 100) as u8));
    let Some(actual) = bookos_shell::brillo_actual() else {
        return;
    };
    let e = &mut state.brillo_auto;
    if let Some(puesta) = e.pantalla_puesta
        && !reciente(e)
        && actual.abs_diff(puesta) > 2
    {
        e.desvio = (e.desvio + i32::from(actual) - i32::from(puesta)).clamp(-60, 60);
        tracing::debug!(desvio = e.desvio, "brillo movido a mano");
    }
    let objetivo = (curva(lux) + e.desvio).clamp(5, 100) as u8;
    let ahora = Instant::now();
    if let Some(rampa) = e.rampa.as_mut() {
        rampa.transicion.dirigir(objetivo, ahora);
        e.pantalla_puesta = Some(objetivo);
        return;
    }
    let desde = if reciente(e) {
        e.a_mano.or(e.pantalla_puesta).unwrap_or(actual)
    } else {
        actual
    };
    if objetivo == desde {
        if !reciente(e) {
            e.pantalla_puesta = Some(actual);
        }
        return;
    }
    let Some((dispositivo, maximo)) = bookos_shell::backlight() else {
        return;
    };
    e.pantalla_puesta = Some(objetivo);
    e.a_mano = None;
    let transicion = Transicion::nueva(f64::from(desde), objetivo, ahora);
    let mut ultimo = None;
    let token = state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            let e = &mut state.brillo_auto;
            let Some(rampa) = e.rampa.as_ref() else {
                return TimeoutAction::Drop;
            };
            let ahora = Instant::now();
            let nivel = rampa.transicion.nivel(ahora);
            let terminada = rampa.transicion.terminada(ahora);
            let crudo = (f64::from(maximo) * nivel / 100.0).round() as u32;
            if ultimo != Some(crudo) {
                bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
                ultimo = Some(crudo);
                e.escrito = Some(ahora);
            }
            if terminada {
                e.rampa = None;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(PASO_RAMPA)
            }
        });
    match token {
        Ok(token) => e.rampa = Some(Rampa { token, transicion }),
        Err(err) => {
            tracing::warn!("no se pudo programar la transición de brillo: {err}");
            if let Some((dispositivo, maximo)) = bookos_shell::backlight() {
                let crudo = (f64::from(maximo) * f64::from(objetivo) / 100.0).round() as u32;
                bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
            }
            e.escrito = Some(ahora);
        }
    }
}

fn teclado(e: &mut Estado, lux: f64) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return;
    };
    if actual > 0 && !reciente(e) {
        e.teclado_encendido = actual;
    }
    let oscuro = if lux < TECLADO_ENCENDER {
        true
    } else if lux > TECLADO_APAGAR {
        false
    } else {
        // Entre los dos umbrales manda la zona de antes; sin zona todavía, lo
        // que haya puesto se respeta.
        match e.teclado_oscuro {
            Some(zona) => zona,
            None => return,
        }
    };
    if e.teclado_oscuro == Some(oscuro) {
        return;
    }
    e.teclado_oscuro = Some(oscuro);
    let nivel = if oscuro {
        e.teclado_encendido.min(maximo)
    } else {
        0
    };
    if nivel != actual {
        bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nivel);
        e.escrito = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_transiciones_suben_y_bajan_sin_saltos_ni_rebasar_el_objetivo() {
        let ahora = Instant::now();
        for (desde, objetivo, duracion) in [
            (20.0, 80, DURACION_SUBIDA),
            (80.0, 20, DURACION_BAJADA),
            (40.0, 41, DURACION_SUBIDA),
            (41.0, 40, DURACION_BAJADA),
        ] {
            let rampa = Transicion::nueva(desde, objetivo, ahora);
            assert_eq!(rampa.nivel(ahora), desde);
            assert_eq!(rampa.duracion, duracion);
            let niveles: Vec<f64> = (0..=100)
                .map(|paso| rampa.nivel(ahora + duracion * paso / 100))
                .collect();
            let fin = f64::from(objetivo);
            assert!(
                niveles
                    .iter()
                    .all(|n| (desde.min(fin)..=desde.max(fin)).contains(n))
            );
            assert!(niveles.windows(2).all(|p| if desde < fin {
                p[0] <= p[1]
            } else {
                p[0] >= p[1]
            }));
            assert_eq!(rampa.nivel(ahora + duracion), fin);
            assert!(rampa.terminada(ahora + duracion));
            assert_eq!(rampa.nivel(ahora + duracion * 2), fin);
            assert!(
                (niveles[1] - desde).abs() < (niveles[50] - niveles[49]).abs(),
                "la transición empieza suavemente"
            );
            assert!(
                (fin - niveles[99]).abs() < (niveles[50] - niveles[49]).abs(),
                "la transición llega suavemente"
            );
        }
    }

    #[test]
    fn una_nueva_lectura_continua_desde_el_nivel_en_curso_aunque_invierta_el_sentido() {
        let ahora = Instant::now();
        let mut rampa = Transicion::nueva(80.0, 20, ahora);
        let cambio = ahora + Duration::from_millis(300);
        let nivel = rampa.nivel(cambio);
        assert!(nivel < 80.0, "la bajada empieza sin esperar tres segundos");
        rampa.dirigir(90, cambio);
        assert_eq!(rampa.nivel(cambio), nivel);
        assert!(rampa.nivel(cambio + PASO_RAMPA) > nivel);
        assert_eq!(rampa.nivel(cambio + DURACION_SUBIDA), 90.0);
    }

    #[test]
    fn las_lecturas_con_el_mismo_objetivo_no_reinician_la_transicion() {
        let ahora = Instant::now();
        let mut rampa = Transicion::nueva(20.0, 80, ahora);
        for paso in 1..=100 {
            rampa.dirigir(80, ahora + DURACION_SUBIDA * paso / 100);
        }
        assert_eq!(rampa.inicio, ahora);
        assert!(rampa.terminada(ahora + DURACION_SUBIDA));
        assert_eq!(rampa.nivel(ahora + DURACION_SUBIDA), 80.0);
    }

    #[test]
    fn sin_animacion_el_objetivo_se_aplica_en_un_solo_paso() {
        let ahora = Instant::now();
        let mut rampa = Transicion::nueva(80.0, 20, ahora);
        rampa.duracion = Duration::ZERO;
        assert_eq!(rampa.nivel(ahora), 20.0);
        assert!(rampa.terminada(ahora));
    }

    #[test]
    fn la_curva_sube_con_la_luz_y_no_se_sale() {
        assert_eq!(curva(0.0), 12);
        assert_eq!(curva(18.0), 40);
        assert!(curva(1000.0) < curva(10_000.0));
        assert!(curva(100_000.0) > 100, "la acota quien la usa, no la curva");
        assert_eq!(
            curva(-3.0),
            12,
            "una lectura negativa cuenta como oscuridad"
        );
    }
}
