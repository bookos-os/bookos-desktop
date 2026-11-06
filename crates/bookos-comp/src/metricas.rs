//! Qué cuesta cada fotograma, en qué se va y cuántos se pierden.
//!
//! Antes de esto lo único que había era una línea de traza cada cinco segundos
//! con los fotogramas enviados y una media, agregada de **todas** las salidas y
//! sin decir en qué se iba el tiempo. Con eso no se puede decidir si el tirón
//! al abrir una actividad es la escena (CPU, iced y tiny-skia), el dibujo (GPU)
//! o que el compositor no llega al vblank: hay que poder verlo separado y por
//! monitor, porque con dos pantallas a distinto refresco el agregado no
//! significa nada.
//!
//! ## Qué se cuenta y qué no
//!
//! - `presentados` se cuenta **en el vblank**, no al encolar: es lo que de
//!   verdad llegó a la pantalla.
//! - `saltados` es damage evaluado y vacío. En reposo esto es lo normal y lo
//!   bueno; no es un fallo.
//! - `aplazados` es damage que llegó con un fotograma en el aire. Un goteo es
//!   normal —el cliente commiteó a mitad de vblank—; muchos por segundo son la
//!   señal de que el compositor va por detrás del monitor.
//! - `perdidos` son fotogramas cuyo coste superó el periodo del modo. Esto no
//!   es una estimación: si componer y dibujar costó más de 8,3 ms a 120 Hz, ese
//!   vblank ya pasó. No se deduce de los números de secuencia de DRM porque en
//!   reposo el hueco entre dos fotogramas es de segundos y no significa nada.
//!
//! Todo se agrega en ventanas de un segundo. Ni el ojo ni el panel de
//! diagnóstico aprovechan más resolución, y una ventana más corta obligaría a
//! repintar el panel más a menudo — el instrumento falsearía la medida.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookos_shell::diagnostico::Datos;

/// Cuánto dura la ventana de agregación.
const VENTANA: Duration = Duration::from_secs(1);
/// Cada cuánto se vuelca el resumen a la traza.
const INFORME: Duration = Duration::from_secs(5);

#[derive(Default, Clone, Copy)]
struct Contadores {
    presentados: u64,
    saltados: u64,
    aplazados: u64,
    perdidos: u64,
    escena: Duration,
    render: Duration,
    total: Duration,
    peor: Duration,
    n: u64,
    // añadido en la 0.2.0 -- la pintura del shell no estaba incluida en escena ni render.
    shell: Duration,
    entrada: Duration,
    entradas: u64,
    abrir: Duration,
    aperturas: u64,
    ultima_apertura: Option<&'static str>,
}

impl Contadores {
    // 0.1.0
    fn suma(&mut self, otro: &Contadores) {
        self.presentados += otro.presentados;
        self.saltados += otro.saltados;
        self.aplazados += otro.aplazados;
        self.perdidos += otro.perdidos;
        self.escena += otro.escena;
        self.render += otro.render;
        self.total += otro.total;
        self.peor = self.peor.max(otro.peor);
        self.n += otro.n;
        self.shell += otro.shell;
        self.entrada += otro.entrada;
        self.entradas += otro.entradas;
        self.abrir += otro.abrir;
        self.aperturas += otro.aperturas;
        self.ultima_apertura = otro.ultima_apertura.or(self.ultima_apertura);
    }

    // 0.1.0
    fn hubo_actividad(&self) -> bool {
        self.presentados > 0 || self.saltados > 0 || self.n > 0
    }
}

/// El modo con el que está encendida una salida. Se apunta al dibujar porque
/// cambia en caliente —conectar un monitor, cambiar de resolución— y el panel
/// tiene que enseñar lo que hay ahora, no lo que había al arrancar.
#[derive(Default, Clone, Copy)]
struct Modo {
    ancho: i32,
    alto: i32,
    escala: f64,
    hz: f32,
    vrr: bool,
}

impl Modo {
    /// Cuánto dura un fotograma a este refresco. Es el plazo que hay que
    /// cumplir; pasarse de aquí es perder el vblank.
    // 0.1.0
    fn periodo(&self) -> Duration {
        if self.hz <= 0.0 {
            // Sin modo conocido no se acusa a nadie de llegar tarde.
            return Duration::MAX;
        }
        Duration::from_secs_f32(1.0 / self.hz)
    }
}

struct Salida {
    nombre: String,
    modo: Modo,
    ventana: Contadores,
    informe: Contadores,
    // añadido en la 0.2.0 -- una entrada posterior al encolado espera otro fotograma.
    entrada_en_vuelo: Option<Instant>,
    aperturas_en_vuelo: Vec<(&'static str, Instant)>,
}

pub struct Metricas {
    salidas: Vec<Salida>,
    desde: Instant,
    ultimo_informe: Instant,
    cpu: Cpu,
    gpu: Option<Gpu>,
    /// La última ventana cerrada, ya en la forma que dibuja el shell.
    datos: Datos,
    // añadido en la 0.2.0
    pintura_pendiente: Duration,
    entrada_pendiente: Option<Instant>,
    entrada_medida: Option<Instant>,
    aperturas_pendientes: Vec<(&'static str, Instant)>,
}

impl Default for Metricas {
    // 0.1.0
    fn default() -> Self {
        Self::new()
    }
}

impl Metricas {
    // 0.1.0
    pub fn new() -> Self {
        let ahora = Instant::now();
        Self {
            salidas: Vec::new(),
            desde: ahora,
            ultimo_informe: ahora,
            cpu: Cpu::new(),
            gpu: Gpu::buscar(),
            datos: Datos::default(),
            pintura_pendiente: Duration::ZERO,
            entrada_pendiente: None,
            entrada_medida: None,
            aperturas_pendientes: Vec::new(),
        }
    }

    // 0.1.0
    fn salida(&mut self, nombre: &str) -> &mut Salida {
        // Lineal a propósito: son como mucho cuatro monitores y esto se llama
        // una vez por fotograma. Un mapa costaría más que el recorrido.
        if let Some(i) = self.salidas.iter().position(|s| s.nombre == nombre) {
            return &mut self.salidas[i];
        }
        self.salidas.push(Salida {
            nombre: nombre.to_string(),
            modo: Modo::default(),
            ventana: Contadores::default(),
            informe: Contadores::default(),
            entrada_en_vuelo: None,
            aperturas_en_vuelo: Vec::new(),
        });
        self.salidas.last_mut().expect("acabo de insertarla")
    }

    // 0.1.0
    pub fn modo(&mut self, nombre: &str, ancho: i32, alto: i32, escala: f64, hz: f32, vrr: bool) {
        self.salida(nombre).modo = Modo {
            ancho,
            alto,
            escala,
            hz,
            vrr,
        };
    }

    /// Se evaluó el damage y no había nada que cambiar.
    // 0.1.0
    pub fn saltado(&mut self, nombre: &str) {
        self.salida(nombre).ventana.saltados += 1;
    }

    /// Había trabajo y la salida tenía un fotograma en el aire.
    // 0.1.0
    pub fn aplazado(&mut self, nombre: &str) {
        self.salida(nombre).ventana.aplazados += 1;
    }

    // añadido en la 0.2.0
    pub fn pintado(&mut self, coste: Duration) {
        self.pintura_pendiente += coste;
    }

    // añadido en la 0.2.0
    pub fn entrada(&mut self, ahora: Instant) {
        self.entrada_pendiente = Some(ahora);
    }

    // añadido en la 0.2.0
    pub fn apertura(&mut self, nombre: &'static str, desde: Instant) {
        self.aperturas_pendientes.push((nombre, desde));
    }

    /// Un fotograma compuesto y encolado, con lo que costó cada parte.
    // 0.1.0
    pub fn dibujado(&mut self, nombre: &str, escena: Duration, render: Duration) {
        let pintura = std::mem::take(&mut self.pintura_pendiente);
        let salida = self.salida(nombre);
        let total = pintura + escena + render;
        let plazo = salida.modo.periodo();
        let c = &mut salida.ventana;
        c.shell += pintura;
        c.escena += escena;
        c.render += render;
        c.total += total;
        c.peor = c.peor.max(total);
        c.n += 1;
        if total > plazo {
            c.perdidos += 1;
        }
    }

    // añadido en la 0.2.0 -- un render fallido o sin damage no puede consumir la latencia pendiente.
    pub fn encolado(&mut self, nombre: &str, emergente: Option<&'static str>) {
        let entrada = self.entrada_pendiente;
        let aperturas = if let Some(emergente) = emergente {
            std::mem::take(&mut self.aperturas_pendientes)
                .into_iter()
                .filter(|(nombre, _)| *nombre == emergente)
                .collect()
        } else {
            Vec::new()
        };
        let salida = self.salida(nombre);
        salida.entrada_en_vuelo = entrada;
        salida.aperturas_en_vuelo = aperturas;
    }

    /// El hardware confirma que enseñó un fotograma.
    // 0.1.0
    pub fn presentado(&mut self, nombre: &str) {
        // modificado en la 0.2.0 -- consume solo las marcas que viajaban en este fotograma.
        let ahora = Instant::now();
        let medida = self.entrada_medida;
        let salida = self.salida(nombre);
        let entrada = salida
            .entrada_en_vuelo
            .take()
            .filter(|t| medida.is_none_or(|m| *t > m));
        let c = &mut salida.ventana;
        c.presentados += 1;
        if let Some(desde) = entrada {
            c.entrada += ahora.saturating_duration_since(desde);
            c.entradas += 1;
        }
        for (emergente, desde) in salida.aperturas_en_vuelo.drain(..) {
            let coste = ahora.saturating_duration_since(desde);
            c.abrir += coste;
            c.aperturas += 1;
            c.ultima_apertura = Some(emergente);
            tracing::info!(
                emergente,
                abrir_ms = ms(coste),
                "primera presentación de emergente"
            );
        }
        if let Some(desde) = entrada {
            self.entrada_medida = Some(desde);
            if self.entrada_pendiente == Some(desde) {
                self.entrada_pendiente = None;
            }
        }
    }

    /// Descarta las salidas que ya no están. Al desconectar un monitor sus
    /// números dejan de tener sentido, y dejarlos en el panel confunde más que
    /// ayuda: parecería una pantalla congelada a 0 fps.
    // 0.1.0
    pub fn retener(&mut self, vivas: &[String]) {
        self.salidas.retain(|s| vivas.contains(&s.nombre));
    }

    /// ¿Ha vencido ya la ventana? Se pregunta antes de cerrarla para no armar
    /// la lista de salidas vivas en cada vuelta del bucle.
    // 0.1.0
    pub fn toca_cerrar(&self) -> bool {
        self.desde.elapsed() >= VENTANA
    }

    // 0.1.0
    pub fn datos(&self) -> &Datos {
        &self.datos
    }

    /// Cierra la ventana si ya toca. Devuelve `true` cuando hay medida nueva.
    ///
    /// Se llama al final de cada vuelta del bucle, así que en reposo puede no
    /// llamarse en varios segundos: la ventana se divide por el tiempo real
    /// transcurrido, no por su duración nominal, o un escritorio quieto
    /// enseñaría un ritmo inventado al despertarse.
    // 0.1.0
    pub fn cerrar_ventana(&mut self) -> bool {
        let ahora = Instant::now();
        let transcurrido = ahora.duration_since(self.desde);
        if transcurrido < VENTANA {
            return false;
        }
        let secs = transcurrido.as_secs_f32().max(f32::EPSILON);
        self.datos = Datos {
            cpu: self.cpu.uso(ahora),
            gpu: self.gpu.as_mut().and_then(|g| g.uso(ahora)),
            ram_mb: rss_propio_mb(),
            salidas: self
                .salidas
                .iter()
                .map(|s| {
                    let c = &s.ventana;
                    let n = c.n.max(1) as f32;
                    bookos_shell::diagnostico::Salida {
                        nombre: s.nombre.clone(),
                        ancho: s.modo.ancho,
                        alto: s.modo.alto,
                        escala: s.modo.escala,
                        hz: s.modo.hz,
                        vrr: s.modo.vrr,
                        fps: c.presentados as f32 / secs,
                        ms_total: ms(c.total) / n,
                        ms_escena: ms(c.escena) / n,
                        ms_render: ms(c.render) / n,
                        shell_ms: ms(c.shell) / n,
                        entrada_ms: (c.entradas > 0).then(|| ms(c.entrada) / c.entradas as f32),
                        abrir_ms: (c.aperturas > 0).then(|| ms(c.abrir) / c.aperturas as f32),
                        emergente: c.ultima_apertura.map(str::to_owned),
                        ms_peor: ms(c.peor),
                        saltados: c.saltados as u32,
                        aplazados: c.aplazados as u32,
                        perdidos: c.perdidos as u32,
                    }
                })
                .collect(),
        };
        for salida in &mut self.salidas {
            let ventana = salida.ventana;
            salida.informe.suma(&ventana);
            salida.ventana = Contadores::default();
        }
        self.desde = ahora;
        self.informar(ahora);
        true
    }

    /// Vuelca a la traza como mucho una vez cada cinco segundos, y solo si hubo
    /// actividad: un escritorio en reposo no debe escribir nada en el registro.
    // 0.1.0
    fn informar(&mut self, ahora: Instant) {
        let transcurrido = ahora.duration_since(self.ultimo_informe);
        if transcurrido < INFORME {
            return;
        }
        let secs = transcurrido.as_secs_f64().max(f64::EPSILON);
        for salida in &mut self.salidas {
            let c = salida.informe;
            if c.hubo_actividad() {
                let n = c.n.max(1) as f32;
                tracing::info!(
                    salida = %salida.nombre,
                    fps = format_args!("{:.1}", c.presentados as f64 / secs),
                    media_ms = format_args!("{:.2}", ms(c.total) / n),
                    escena_ms = format_args!("{:.2}", ms(c.escena) / n),
                    render_ms = format_args!("{:.2}", ms(c.render) / n),
                    shell_ms = format_args!("{:.2}", ms(c.shell) / n),
                    entrada_ms = (c.entradas > 0).then(|| ms(c.entrada) / c.entradas as f32),
                    abrir_ms = (c.aperturas > 0).then(|| ms(c.abrir) / c.aperturas as f32),
                    emergente = c.ultima_apertura,
                    peor_ms = format_args!("{:.2}", ms(c.peor)),
                    saltados = c.saltados,
                    aplazados = c.aplazados,
                    perdidos = c.perdidos,
                    "ritmo de dibujo"
                );
            }
            salida.informe = Contadores::default();
        }
        self.ultimo_informe = ahora;
    }
}

// 0.1.0
fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

/// Cuánta CPU consume **este proceso**.
///
/// Se mide el compositor y no la máquina a propósito: lo que hace falta saber
/// al mirar el panel es cuánto se puede achacar a este código. El resto de la
/// carga del sistema no la va a arreglar el compositor.
struct Cpu {
    ticks: u64,
    desde: Instant,
}

impl Cpu {
    // 0.1.0
    fn new() -> Self {
        Self {
            ticks: ticks_propios().unwrap_or(0),
            desde: Instant::now(),
        }
    }

    /// Porcentaje de **un núcleo**: 100 % es un hilo saturado, y con varios
    /// hilos puede pasar de 100. Es la misma escala que enseña `top`.
    // 0.1.0
    fn uso(&mut self, ahora: Instant) -> f32 {
        let Some(ticks) = ticks_propios() else {
            return 0.0;
        };
        let secs = ahora.duration_since(self.desde).as_secs_f32();
        let usados = ticks.saturating_sub(self.ticks);
        self.ticks = ticks;
        self.desde = ahora;
        if secs <= 0.0 {
            return 0.0;
        }
        // USER_HZ es 100 en Linux para lo que sale por /proc, independientemente
        // de CONFIG_HZ: el kernel convierte antes de exportarlo. Por eso no hace
        // falta `sysconf(_SC_CLK_TCK)` ni, con él, una dependencia de libc.
        const USER_HZ: f32 = 100.0;
        usados as f32 / USER_HZ / secs * 100.0
    }
}

/// `utime + stime` de /proc/self/stat, en ticks.
// 0.1.0
fn ticks_propios() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // El segundo campo es el nombre del ejecutable entre paréntesis y puede
    // llevar espacios dentro, así que no se puede partir por espacios desde el
    // principio: se corta después del último ')' y se cuenta desde ahí.
    let resto = &stat[stat.rfind(')')? + 1..];
    let mut campos = resto.split_whitespace();
    // Tras el ')' viene `state`, que es el campo 3: utime es el 14 y stime el
    // 15, o sea el 11.º y el 12.º de lo que queda.
    let utime: u64 = campos.nth(10)?.parse().ok()?;
    let stime: u64 = campos.next()?.parse().ok()?;
    Some(utime + stime)
}

/// Memoria residente del compositor, de `VmRSS`. Es la cifra que enseña `top`
/// como RES: cuenta lo compartido con otras bibliotecas, pero es estable y no
/// cuesta más que leer un archivo una vez por segundo.
// añadido en la 0.2.0
fn rss_propio_mb() -> Option<f32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f32 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb / 1024.0)
}

/// De dónde sale la ocupación de la GPU. Cada driver la publica a su manera, y
/// si no hay ninguna de las dos el panel enseña una raya en vez de inventarla.
// modificado en la 0.2.0 -- antes solo `gpu_busy_percent`, que `xe` no tiene.
enum Gpu {
    /// `amdgpu`: el porcentaje ya calculado.
    Ocupacion(PathBuf),
    /// `xe` (Lunar Lake): solo publica cuánto lleva dormida la parte de
    /// gráficos (RC6), así que la ocupación es lo que no durmió. Es una cota
    /// por arriba: despierta no siempre es trabajando. Medido en reposo con el
    /// escritorio quieto: entre un 17 y un 52 % despierta.
    Reposo {
        ruta: PathBuf,
        ms: u64,
        desde: Instant,
    },
}

impl Gpu {
    // modificado en la 0.2.0
    fn buscar() -> Option<Self> {
        let tarjetas: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm")
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path().join("device"))
            .collect();
        if let Some(ruta) = tarjetas
            .iter()
            .map(|d| d.join("gpu_busy_percent"))
            .find(|p| p.exists())
        {
            return Some(Self::Ocupacion(ruta));
        }
        // gt0 es la de gráficos; gt1, la de vídeo, no la usa el compositor.
        let ruta = tarjetas
            .iter()
            .map(|d| d.join("tile0/gt0/gtidle/idle_residency_ms"))
            .find(|p| p.exists())?;
        Some(Self::Reposo {
            ms: leer_numero(&ruta)?,
            ruta,
            desde: Instant::now(),
        })
    }

    // añadido en la 0.2.0
    fn uso(&mut self, ahora: Instant) -> Option<f32> {
        match self {
            Self::Ocupacion(ruta) => std::fs::read_to_string(ruta).ok()?.trim().parse().ok(),
            Self::Reposo { ruta, ms, desde } => {
                let nuevo = leer_numero(ruta)?;
                let dormida = nuevo.saturating_sub(*ms) as f32;
                let total = self::ms(ahora.duration_since(*desde));
                *ms = nuevo;
                *desde = ahora;
                (total > 0.0).then(|| (100.0 - dormida / total * 100.0).clamp(0.0, 100.0))
            }
        }
    }
}

// añadido en la 0.2.0
fn leer_numero(ruta: &PathBuf) -> Option<u64> {
    std::fs::read_to_string(ruta).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // añadido en la 0.2.0
    #[test]
    fn un_vblank_anterior_no_consume_la_entrada_nueva() {
        let mut m = Metricas::new();
        m.encolado("panel", None);
        let evento = Instant::now();
        m.entrada(evento);
        m.presentado("panel");
        assert_eq!(m.salida("panel").ventana.entradas, 0);
        m.dibujado("panel", Duration::ZERO, Duration::ZERO);
        m.presentado("panel");
        assert_eq!(
            m.salida("panel").ventana.entradas,
            0,
            "dibujar no confirma el encolado"
        );
        m.encolado("panel", None);
        m.presentado("panel");
        assert_eq!(m.salida("panel").ventana.entradas, 1);
        assert!(m.entrada_pendiente.is_none());
    }

    // añadido en la 0.2.0
    #[test]
    fn dos_monitores_no_duplican_una_misma_latencia() {
        let mut m = Metricas::new();
        m.entrada(Instant::now());
        m.encolado("primero", None);
        m.encolado("segundo", None);
        m.presentado("segundo");
        m.presentado("primero");
        assert_eq!(
            m.salida("primero").ventana.entradas + m.salida("segundo").ventana.entradas,
            1
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn abrir_se_mide_en_el_fotograma_que_contiene_la_tarjeta() {
        let mut m = Metricas::new();
        m.apertura("centro", Instant::now());
        m.encolado("secundario", None);
        m.presentado("secundario");
        assert_eq!(m.salida("secundario").ventana.aperturas, 0);
        m.encolado("principal", Some("centro"));
        m.presentado("principal");
        assert_eq!(m.salida("principal").ventana.aperturas, 1);
        assert_eq!(
            m.salida("principal").ventana.ultima_apertura,
            Some("centro")
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn pintar_shell_cuenta_en_el_presupuesto_del_fotograma() {
        let mut m = Metricas::new();
        m.modo("panel", 2880, 1800, 1.75, 120.0, false);
        m.pintado(Duration::from_millis(9));
        m.dibujado("panel", Duration::ZERO, Duration::ZERO);
        assert_eq!(m.salida("panel").ventana.perdidos, 1);
        m.dibujado("panel", Duration::ZERO, Duration::ZERO);
        assert_eq!(m.salida("panel").ventana.shell, Duration::from_millis(9));
    }

    /// Un fotograma que se pasa del periodo del modo es un vblank perdido, y
    /// uno que cabe no lo es. Es la única cuenta de este módulo que decide algo.
    #[test]
    fn el_plazo_lo_marca_el_refresco() {
        let mut m = Metricas::new();
        m.modo("eDP-1", 2880, 1800, 1.75, 120.0, false);
        // 8,3 ms de plazo a 120 Hz.
        m.dibujado(
            "eDP-1",
            Duration::from_micros(3000),
            Duration::from_micros(2000),
        );
        m.dibujado(
            "eDP-1",
            Duration::from_micros(6000),
            Duration::from_micros(5000),
        );
        assert_eq!(m.salida("eDP-1").ventana.perdidos, 1);
        assert_eq!(m.salida("eDP-1").ventana.n, 2);
    }

    /// Sin modo conocido no se acusa a nadie: al arrancar todavía no hay
    /// refresco apuntado y todos los fotogramas parecerían perdidos.
    #[test]
    fn sin_modo_no_hay_fotogramas_perdidos() {
        let mut m = Metricas::new();
        m.dibujado("eDP-1", Duration::from_millis(80), Duration::ZERO);
        assert_eq!(m.salida("eDP-1").ventana.perdidos, 0);
    }

    /// El proceso siempre ha consumido alguna CPU, y la cuenta no puede salir
    /// negativa ni disparatada.
    #[test]
    fn la_ram_del_proceso_se_lee() {
        assert!(rss_propio_mb().is_some_and(|mb| mb > 0.0));
    }

    #[test]
    fn la_cpu_del_proceso_se_lee() {
        let ticks = ticks_propios().expect("/proc/self/stat existe en Linux");
        assert!(ticks < 1_000_000, "ticks disparatados: {ticks}");
    }
}
