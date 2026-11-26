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

const PASO_RAMPA: Duration = Duration::from_millis(8);
// Al oscurecer hay tiempo para adaptar la vista; al salir a la luz hace falta
// recuperar antes la legibilidad. Se anima con muelle críticamente amortiguado
// continuo (C1) en lugar de curvas reiniciables.
const RESPUESTA_BAJADA: f64 = 1.60;
const RESPUESTA_SUBIDA: f64 = 0.80;
const TOLERANCIA_NIVEL: f64 = 0.08;
const TOLERANCIA_VELOCIDAD: f64 = 0.15;
/// Histéresis al arrancar desde reposo: ignora fluctuaciones de ±1 % del sensor.
const HISTERESIS_INICIO: u8 = 2;

fn debe_iniciar_rampa(desde: u8, objetivo: u8) -> bool {
    desde.abs_diff(objetivo) >= HISTERESIS_INICIO
}

struct Transicion {
    origen: f64,
    velocidad: f64,
    objetivo: u8,
    inicio: Instant,
    frecuencia: f64,
    instantanea: bool,
}

impl Transicion {
    fn frecuencia_para(desde: f64, objetivo: u8) -> f64 {
        let respuesta = if f64::from(objetivo) < desde {
            RESPUESTA_BAJADA
        } else {
            RESPUESTA_SUBIDA
        };
        std::f64::consts::TAU / respuesta
    }

    fn con_efectos(
        desde: f64,
        velocidad: f64,
        objetivo: u8,
        ahora: Instant,
        animar: bool,
    ) -> Self {
        Self {
            origen: desde,
            velocidad,
            objetivo,
            inicio: ahora,
            frecuencia: Self::frecuencia_para(desde, objetivo),
            instantanea: !animar,
        }
    }

    // 0.1.1
    // modificado en la 0.2.3 -- muelle críticamente amortiguado en forma cerrada
    fn nueva(desde: f64, objetivo: u8, ahora: Instant) -> Self {
        Self::con_efectos(
            desde,
            0.0,
            objetivo,
            ahora,
            !bookos_shell::tema::efectos_reducidos(),
        )
    }

    fn estado(&self, ahora: Instant) -> (f64, f64) {
        let fin = f64::from(self.objetivo);
        if self.instantanea {
            return (fin, 0.0);
        }
        let t = ahora.saturating_duration_since(self.inicio).as_secs_f64();
        let w = self.frecuencia;
        let c1 = self.origen - fin;
        let c2 = self.velocidad + w * c1;
        let e = (-w * t).exp();
        (fin + (c1 + c2 * t) * e, (c2 - w * (c1 + c2 * t)) * e)
    }

    // 0.1.1
    fn terminada(&self, ahora: Instant) -> bool {
        if self.instantanea {
            return true;
        }
        let (nivel, velocidad) = self.estado(ahora);
        (nivel - f64::from(self.objetivo)).abs() < TOLERANCIA_NIVEL
            && velocidad.abs() < TOLERANCIA_VELOCIDAD
    }

    // 0.1.1
    fn nivel(&self, ahora: Instant) -> f64 {
        if self.terminada(ahora) {
            return f64::from(self.objetivo);
        }
        self.estado(ahora).0.clamp(0.0, 100.0)
    }

    // 0.1.1
    // modificado en la 0.2.3 -- conserva velocidad continua C1 en el mismo sentido sin rebasar
    fn dirigir(&mut self, objetivo: u8, ahora: Instant) {
        if self.objetivo == objetivo {
            return;
        }
        let (nivel, vel_actual) = self.estado(ahora);
        let frecuencia = Self::frecuencia_para(nivel, objetivo);
        let delta = f64::from(objetivo) - nivel;
        let velocidad = if delta * vel_actual > 0.0 {
            let limite = frecuencia * delta.abs();
            vel_actual.clamp(-limite, limite)
        } else {
            0.0
        };
        *self = Self {
            origen: nivel,
            velocidad,
            objetivo,
            inicio: ahora,
            frecuencia,
            instantanea: self.instantanea,
        };
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
    escrito_pantalla: Option<Instant>,
    escrito_teclado: Option<Instant>,
    /// Último nivel pedido con la tecla. Mientras sysfs no lo refleje, la
    /// siguiente pulsación parte de aquí y no de la lectura atrasada.
    a_mano: Option<u8>,
    /// Último nivel de teclado pedido. Sus LEDs también tardan en reflejar la
    /// escritura asíncrona, pero tienen escala propia y no comparten el nivel.
    teclado_a_mano: Option<(String, u32, Instant)>,
    rampa: Option<Rampa>,
}

fn crear_proxy_brillo() -> zbus::Result<zbus::blocking::Proxy<'static>> {
    use zbus::blocking::{Connection, Proxy};
    let conexion = Connection::system()?;
    zbus::blocking::proxy::Builder::<Proxy>::new(&conexion)
        .destination("org.freedesktop.login1")?
        .path("/org/freedesktop/login1/session/auto")?
        .interface("org.freedesktop.login1.Session")?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

/// Instala un proxy D-Bus persistente hacia `logind` para evitar lanzar un
/// subproceso `busctl` en cada paso de la rampa a 120 Hz.
fn instalar_escritor_zbus() {
    let proxy: std::sync::Mutex<Option<zbus::blocking::Proxy<'static>>> =
        std::sync::Mutex::new(None);
    bookos_shell::retroiluminacion::instalar_escritor(move |subsistema, dispositivo, nivel| {
        let mut cache = proxy.lock().unwrap_or_else(|e| e.into_inner());
        for intento in 0..2 {
            if cache.is_none() {
                *cache = Some(
                    crear_proxy_brillo()
                        .map_err(|err| format!("no se pudo conectar con login1: {err}"))?,
                );
            }
            let p = cache.as_ref().expect("proxy recién creado");
            match p.call_method("SetBrightness", &(subsistema, dispositivo, nivel)) {
                Ok(_) => return Ok(()),
                Err(err) if intento == 0 => {
                    tracing::debug!("reintentando SetBrightness tras error de proxy: {err}");
                    *cache = None;
                }
                Err(err) => {
                    *cache = None;
                    return Err(err.to_string());
                }
            }
        }
        Err("no se pudo escribir el brillo".into())
    });
}

impl Estado {
    // 0.1.0
    // modified 0.1.1 -- sin la espera al bajar: la sustituye la rampa
    pub fn arrancar(
        loop_handle: &LoopHandle<'static, BookosComp>,
        elegido: bookos_shell::BrilloAutomatico,
    ) -> anyhow::Result<Self> {
        instalar_escritor_zbus();
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
            escrito_pantalla: None,
            escrito_teclado: None,
            a_mano: None,
            teclado_a_mano: None,
            rampa: None,
        };
        estado.reclamar_si_hace_falta();
        Ok(estado)
    }

    // 0.1.0
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

    fn reciente_pantalla(&self) -> bool {
        self.rampa.is_some()
            || self
                .escrito_pantalla
                .is_some_and(|t| t.elapsed() < MARGEN_ESCRITURA)
    }

    fn reciente_teclado(&self) -> bool {
        self.escrito_teclado
            .is_some_and(|t| t.elapsed() < MARGEN_ESCRITURA)
    }

    fn a_mano_reciente(&self) -> Option<u8> {
        match (self.a_mano, self.escrito_pantalla) {
            (Some(nivel), Some(t)) if t.elapsed() < MARGEN_ESCRITURA => Some(nivel),
            _ => None,
        }
    }
}

/// El hilo del bus: reclama o suelta el sensor según las órdenes y reenvía cada
/// lectura en lux.
// 0.1.0
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
// 0.1.0
// modified 0.1.1 -- la rampa guarda su token junto a su estado
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
// 0.1.0
pub fn partida_tecla(state: &BookosComp) -> Option<u8> {
    state
        .brillo_auto
        .a_mano_reciente()
        .or_else(bookos_shell::brillo_actual)
}

/// La tecla ha pedido `nivel`. El automático lo aprende como desvío en el acto
/// y suelta su rampa: si no, la rampa en curso seguía escribiendo su objetivo
/// encima del de la tecla y el brillo bailaba al soltar.
// 0.1.0
// modified 0.1.1 -- la rampa guarda su token junto a su estado
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
    e.escrito_pantalla = Some(Instant::now());
}

// 0.1.0
pub fn partida_tecla_teclado(state: &BookosComp, dispositivo: &str, actual: u32) -> u32 {
    state
        .brillo_auto
        .teclado_a_mano
        .as_ref()
        .filter(|(d, _, t)| d == dispositivo && t.elapsed() < MARGEN_ESCRITURA)
        .map_or(actual, |(_, nivel, _)| *nivel)
}

// 0.1.0
pub fn tecla_teclado(state: &mut BookosComp, dispositivo: &str, nivel: u32) {
    state.brillo_auto.teclado_a_mano = Some((dispositivo.to_owned(), nivel, Instant::now()));
    state.brillo_auto.escrito_teclado = Some(Instant::now());
}

// 0.1.0
// modified 0.1.1 -- evalúa en cada lectura (la rampa suaviza) y descarta lecturas no finitas
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
// 0.1.0
// modified 0.1.1 -- la rampa guarda su token junto a su estado
pub fn fijar(state: &mut BookosComp, nivel: u8) {
    let e = &mut state.brillo_auto;
    if let Some(rampa) = e.rampa.take() {
        state.loop_handle.remove(rampa.token);
    }
    if e.pantalla_puesta.is_some() {
        e.pantalla_puesta = Some(nivel);
    }
    e.a_mano = Some(nivel);
    e.escrito_pantalla = Some(Instant::now());
}

/// Ajusta lo que esté en automático a la última lectura.
///
/// Se llama también al volver de DPMS: con la pantalla apagada no se ajusta
/// nada, y la luz pudo cambiar entretanto.
// 0.1.0
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
// 0.1.0
fn curva(lux: f64) -> i32 {
    (12.0 + 22.0 * (1.0 + lux.max(0.0)).log10()).round() as i32
}

fn tarjeta_brillo_abierta(state: &BookosComp) -> bool {
    state
        .shell
        .as_ref()
        .and_then(|s| s.emergente_nombre())
        .is_some_and(|n| matches!(n, "brillo" | "centro"))
}

fn refrescar_tarjeta_brillo(state: &mut BookosComp) {
    if tarjeta_brillo_abierta(state)
        && state
            .shell
            .as_mut()
            .is_some_and(|s| s.refresh_emergente())
    {
        state.needs_redraw = true;
    }
}

fn programar_refresco_final(state: &mut BookosComp) {
    if !tarjeta_brillo_abierta(state) {
        return;
    }
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(30)),
        |_, _, state| {
            refrescar_tarjeta_brillo(state);
            TimeoutAction::Drop
        },
    );
}

// 0.1.0
// modified 0.1.1 -- transición por tiempo que se redirige si llega otra lectura, en vez de pasos de 1/8
// modificado en la 0.2.3 -- arranque fraccional, histéresis en reposo y refresco en caliente de la tarjeta
fn pantalla(state: &mut BookosComp, lux: f64) {
    bookos_shell::retroiluminacion::poner_objetivo_sensor(Some(curva(lux).clamp(5, 100) as u8));
    let Some(actual_exacto) = bookos_shell::brillo_exacto() else {
        return;
    };
    let actual = actual_exacto.round().clamp(0.0, 100.0) as u8;
    let e = &mut state.brillo_auto;
    if let Some(puesta) = e.pantalla_puesta
        && !e.reciente_pantalla()
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
    let (desde_u8, desde_exacto) = if e.reciente_pantalla() {
        let nivel = e.a_mano.or(e.pantalla_puesta).unwrap_or(actual);
        (nivel, f64::from(nivel))
    } else {
        (actual, actual_exacto)
    };
    if !debe_iniciar_rampa(desde_u8, objetivo) {
        if !e.reciente_pantalla() {
            e.pantalla_puesta = Some(actual);
        }
        return;
    }
    let Some((dispositivo, maximo)) = bookos_shell::backlight() else {
        return;
    };
    e.pantalla_puesta = Some(objetivo);
    e.a_mano = None;
    let transicion = Transicion::nueva(desde_exacto, objetivo, ahora);
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
            let cambio = ultimo != Some(crudo);
            if cambio {
                bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
                ultimo = Some(crudo);
                e.escrito_pantalla = Some(ahora);
            }
            if terminada {
                e.rampa = None;
                if cambio {
                    refrescar_tarjeta_brillo(state);
                }
                programar_refresco_final(state);
                TimeoutAction::Drop
            } else {
                if cambio {
                    refrescar_tarjeta_brillo(state);
                }
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
            e.escrito_pantalla = Some(ahora);
        }
    }
}

// 0.1.0
fn teclado(e: &mut Estado, lux: f64) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return;
    };
    if actual > 0 && !e.reciente_teclado() {
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
        e.escrito_teclado = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_transiciones_suben_y_bajan_sin_saltos_ni_rebasar_el_objetivo() {
        let ahora = Instant::now();
        let duracion = Duration::from_secs(4);
        for (desde, objetivo) in [(20.0, 80), (80.0, 20), (40.0, 42), (42.0, 40)] {
            let rampa = Transicion::con_efectos(desde, 0.0, objetivo, ahora, true);
            assert_eq!(rampa.nivel(ahora), desde);
            let niveles: Vec<f64> = (0..=100)
                .map(|paso| rampa.nivel(ahora + Duration::from_millis(2000) * paso / 100))
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
            let max_paso = niveles
                .windows(2)
                .map(|p| (p[1] - p[0]).abs())
                .fold(0.0_f64, f64::max);
            assert!(
                (niveles[1] - desde).abs() < max_paso,
                "la transición empieza suavemente"
            );
            assert!(
                (fin - niveles[99]).abs() < max_paso,
                "la transición llega suavemente"
            );
        }
    }

    #[test]
    fn una_nueva_lectura_continua_desde_el_nivel_en_curso_aunque_invierta_el_sentido() {
        let ahora = Instant::now();
        let mut rampa = Transicion::con_efectos(80.0, 0.0, 20, ahora, true);
        let cambio = ahora + Duration::from_millis(300);
        let nivel = rampa.nivel(cambio);
        assert!(nivel < 80.0, "la bajada empieza sin esperar tres segundos");
        rampa.dirigir(90, cambio);
        assert!((rampa.nivel(cambio) - nivel).abs() < 1e-9);
        assert!(rampa.nivel(cambio + PASO_RAMPA) > nivel);
        assert!(rampa.terminada(cambio + Duration::from_secs(4)));
        assert_eq!(rampa.nivel(cambio + Duration::from_secs(4)), 90.0);
    }

    #[test]
    fn las_lecturas_con_el_mismo_objetivo_no_reinician_la_transicion() {
        let ahora = Instant::now();
        let mut rampa = Transicion::con_efectos(20.0, 0.0, 80, ahora, true);
        for paso in 1..=100 {
            rampa.dirigir(80, ahora + Duration::from_millis(1200) * paso / 100);
        }
        assert_eq!(rampa.inicio, ahora);
        assert!(rampa.terminada(ahora + Duration::from_secs(4)));
        assert_eq!(rampa.nivel(ahora + Duration::from_secs(4)), 80.0);
    }

    #[test]
    fn dirigir_en_el_mismo_sentido_conserva_la_velocidad_sin_sobrepaso() {
        let ahora = Instant::now();
        let mut expandir = Transicion::con_efectos(20.0, 0.0, 60, ahora, true);
        let cambio = ahora + Duration::from_millis(250);
        let (nivel_antes, vel_antes) = expandir.estado(cambio);
        assert!(vel_antes > 0.0);
        expandir.dirigir(90, cambio);
        let (nivel_despues, vel_despues) = expandir.estado(cambio);
        assert!((nivel_despues - nivel_antes).abs() < 1e-9);
        assert!(
            (vel_despues - vel_antes).abs() < 1e-9,
            "conserva la velocidad exacta C1"
        );
        assert!((0..=100).all(|i| {
            let n = expandir.nivel(cambio + Duration::from_millis(20) * i);
            (nivel_antes..=90.0).contains(&n)
        }));

        // Acercar el objetivo en pleno vuelo nunca rebasa el nuevo destino.
        let mut acercar = Transicion::con_efectos(10.0, 0.0, 95, ahora, true);
        let t_corte = ahora + Duration::from_millis(220);
        let nivel_corte = acercar.nivel(t_corte);
        let nuevo_obj = (nivel_corte.ceil() as u8 + 2).min(95);
        acercar.dirigir(nuevo_obj, t_corte);
        assert!((0..=150).all(|i| {
            let n = acercar.nivel(t_corte + Duration::from_millis(16) * i);
            n >= nivel_corte - 1e-9 && n <= f64::from(nuevo_obj) + 1e-9
        }));
    }

    #[test]
    fn la_bajada_es_mas_pausada_que_la_subida() {
        let ahora = Instant::now();
        let subida = Transicion::con_efectos(20.0, 0.0, 80, ahora, true);
        let bajada = Transicion::con_efectos(80.0, 0.0, 20, ahora, true);
        let t = ahora + Duration::from_millis(400);
        let avance_subida = subida.nivel(t) - 20.0;
        let avance_bajada = 80.0 - bajada.nivel(t);
        assert!(avance_subida > avance_bajada);
    }

    #[test]
    fn la_histeresis_en_reposo_ignora_cambios_menores_de_dos_puntos() {
        assert!(!debe_iniciar_rampa(40, 40));
        assert!(!debe_iniciar_rampa(40, 41));
        assert!(!debe_iniciar_rampa(40, 39));
        assert!(debe_iniciar_rampa(40, 42));
        assert!(debe_iniciar_rampa(40, 38));
    }

    #[test]
    fn sin_animacion_el_objetivo_se_aplica_en_un_solo_paso() {
        let ahora = Instant::now();
        let rampa = Transicion::con_efectos(80.0, 0.0, 20, ahora, false);
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

    #[test]
    fn escrito_teclado_no_interfiere_con_reciente_ni_a_mano_pantalla() {
        let (tx, _rx) = mpsc::channel();
        let viejo = Instant::now() - MARGEN_ESCRITURA - Duration::from_secs(1);
        let estado = Estado {
            elegido: bookos_shell::BrilloAutomatico::default(),
            ordenes: tx,
            reclamado: false,
            lux: None,
            pantalla_puesta: None,
            desvio: 0,
            teclado_oscuro: None,
            teclado_encendido: 1,
            escrito_pantalla: Some(viejo),
            escrito_teclado: Some(Instant::now()),
            a_mano: Some(77),
            teclado_a_mano: None,
            rampa: None,
        };
        assert!(!estado.reciente_pantalla());
        assert!(estado.reciente_teclado());
        assert_eq!(estado.a_mano_reciente(), None);
    }
}
