//! La luz nocturna: teñir la pantalla de cálido por la noche.
//!
//! ## Por qué la curva de gamma del CRTC y no un sombreador
//!
//! El tinte se aplica en el último paso del hardware: la tabla de gamma que el
//! controlador de pantalla aplica a cada píxel ya compuesto, camino del panel.
//! Cuesta **cero** por fotograma —no hay una pasada más de GPU a 2880×1800 y
//! 120 Hz—, no rompe el escaneo directo de una ventana a pantalla completa y
//! tiñe también el cursor, que va en su propio plano y un sombreador no vería.
//! Y como no toca lo compuesto, las capturas y la pantalla compartida salen con
//! sus colores de verdad, que es lo que se espera de ellas.
//!
//! El precio: solo existe en la sesión real. Anidado, la pantalla es del
//! compositor anfitrión y no hay curva que tocar; ahí la configuración se
//! guarda y se ve en las tarjetas, pero no tiñe nada.
//!
//! ## El horario: un despertar por cambio
//!
//! Como el tema automático (ver [`crate::apariencia`]): se calcula cuánto falta
//! para el próximo borde del horario y se programa **ese** instante.
//!
//! ## Encenderla a mano con el horario puesto
//!
//! La baldosa no cambia el modo: quien tiene un horario y la apaga una noche no
//! quiere perderlo. Se apunta como forzada y el horario vuelve a mandar en el
//! siguiente borde —apagarla a las once deja de valer a las siete—, que es lo
//! que hace Night Shift.

use std::time::Instant;

use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use bookos_shell::{LuzNocturna, ModoLuzNocturna};

use crate::state::BookosComp;

/// Ritmo del muelle en mireds (`10^6 / K`), perceptualmente uniforme a lo largo
/// de la curva del cuerpo negro: calienta pausado estilo Night Shift y vuelve
/// al blanco neutro con algo más de agilidad.
const RESPUESTA_CALIDO: f64 = 0.85;
const RESPUESTA_NEUTRO: f64 = 0.65;
const TOLERANCIA_MIRED: f64 = 0.15;
const TOLERANCIA_VELOCIDAD: f64 = 0.5;

/// Por lo mismo que en el tema: el cálculo va en minutos y el despertar cae
/// siempre después del minuto en punto.
const MINIMO: std::time::Duration = std::time::Duration::from_secs(70);

fn a_mired(kelvin: f64) -> f64 {
    1_000_000.0 / kelvin.max(1.0)
}

fn desde_mired(mired: f64) -> f64 {
    1_000_000.0 / mired.max(1.0)
}

struct Transicion {
    origen: f64,
    velocidad: f64,
    objetivo: f64,
    objetivo_kelvin: u32,
    inicio: Instant,
    frecuencia: f64,
    instantanea: bool,
}

impl Transicion {
    fn frecuencia_para(desde_mired: f64, objetivo_mired: f64) -> f64 {
        let respuesta = if objetivo_mired > desde_mired {
            RESPUESTA_CALIDO
        } else {
            RESPUESTA_NEUTRO
        };
        std::f64::consts::TAU / respuesta
    }

    fn con_efectos(
        desde_kelvin: f64,
        velocidad_mired: f64,
        objetivo_kelvin: u32,
        ahora: Instant,
        animar: bool,
    ) -> Self {
        let origen = a_mired(desde_kelvin);
        let objetivo = a_mired(f64::from(objetivo_kelvin));
        Self {
            origen,
            velocidad: velocidad_mired,
            objetivo,
            objetivo_kelvin,
            inicio: ahora,
            frecuencia: Self::frecuencia_para(origen, objetivo),
            instantanea: !animar,
        }
    }

    fn nueva(desde_kelvin: f64, objetivo_kelvin: u32, ahora: Instant) -> Self {
        Self::con_efectos(
            desde_kelvin,
            0.0,
            objetivo_kelvin,
            ahora,
            !bookos_shell::tema::efectos_reducidos(),
        )
    }

    fn estado(&self, ahora: Instant) -> (f64, f64) {
        if self.instantanea {
            return (self.objetivo, 0.0);
        }
        let t = ahora.saturating_duration_since(self.inicio).as_secs_f64();
        let w = self.frecuencia;
        let c1 = self.origen - self.objetivo;
        let c2 = self.velocidad + w * c1;
        let e = (-w * t).exp();
        (
            self.objetivo + (c1 + c2 * t) * e,
            (c2 - w * (c1 + c2 * t)) * e,
        )
    }

    fn terminada(&self, ahora: Instant) -> bool {
        if self.instantanea {
            return true;
        }
        let (mired, velocidad) = self.estado(ahora);
        (mired - self.objetivo).abs() < TOLERANCIA_MIRED && velocidad.abs() < TOLERANCIA_VELOCIDAD
    }

    fn kelvin(&self, ahora: Instant) -> f64 {
        if self.terminada(ahora) {
            return f64::from(self.objetivo_kelvin);
        }
        let mired = self.estado(ahora).0;
        let minimo = self.origen.min(self.objetivo);
        let maximo = self.origen.max(self.objetivo);
        desde_mired(mired.clamp(minimo, maximo))
    }

    fn dirigir(&mut self, objetivo_kelvin: u32, ahora: Instant) {
        if self.objetivo_kelvin == objetivo_kelvin {
            return;
        }
        let (mired, vel_actual) = self.estado(ahora);
        let objetivo = a_mired(f64::from(objetivo_kelvin));
        let frecuencia = Self::frecuencia_para(mired, objetivo);
        let delta = objetivo - mired;
        let velocidad = if delta * vel_actual > 0.0 {
            let limite = frecuencia * delta.abs();
            vel_actual.clamp(-limite, limite)
        } else {
            0.0
        };
        *self = Self {
            origen: mired,
            velocidad,
            objetivo,
            objetivo_kelvin,
            inicio: ahora,
            frecuencia,
            instantanea: self.instantanea,
        };
    }
}

pub struct Estado {
    pub config: LuzNocturna,
    /// La temperatura que hay puesta en la curva, en Kelvin. [`LuzNocturna::NEUTRA`] es
    /// sin teñir.
    aplicada: f64,
    /// Encendida o apagada a mano con el horario puesto; vale hasta el próximo
    /// borde del horario.
    forzada: Option<bool>,
    /// El despertar del próximo borde del horario.
    tick: Option<RegistrationToken>,
    /// El fundido en marcha.
    transicion: Option<Transicion>,
    /// Multiplicadores RGB listos para enviar en el próximo fotograma KMS.
    pendiente: Option<[f32; 3]>,
}

impl Estado {
    // 0.1.0
    pub fn nuevo(config: LuzNocturna) -> Self {
        Self {
            config,
            aplicada: f64::from(LuzNocturna::NEUTRA),
            forzada: None,
            tick: None,
            transicion: None,
            pendiente: None,
        }
    }

    pub fn animando(&self) -> bool {
        self.transicion.is_some()
    }

    pub fn tomar_pendiente(&mut self) -> Option<[f32; 3]> {
        self.pendiente.take()
    }

    // 0.1.0
    fn encendida(&self, ahora: bookos_shell::tema::HoraDelDia) -> bool {
        self.forzada.unwrap_or_else(|| self.config.toca(ahora))
    }

    /// La temperatura a la que hay que ir ahora.
    // 0.1.0
    fn objetivo(&self, ahora: bookos_shell::tema::HoraDelDia) -> u32 {
        if self.encendida(ahora) {
            self.config.temperatura
        } else {
            LuzNocturna::NEUTRA
        }
    }
}

/// Lo que se multiplica cada canal a esa temperatura, normalizado para que
/// 6500 K sea `[1, 1, 1]`.
///
/// Es la aproximación de Tanner Helland a la curva del cuerpo negro: tres
/// tramos ajustados sobre los datos de Mitchell Charity, con un error de un par
/// de unidades sobre 255 entre 1000 y 40 000 K. Para teñir una pantalla es de
/// sobra; la tabla exacta de redshift serían doscientas líneas de números para
/// una diferencia que no se ve.
// 0.1.0
pub fn multiplicadores(kelvin: u32) -> [f32; 3] {
    multiplicadores_f64(f64::from(kelvin))
}

fn multiplicadores_f64(kelvin: f64) -> [f32; 3] {
    // 0.1.0
    fn crudo(kelvin: f64) -> [f32; 3] {
        let t = kelvin as f32 / 100.0;
        let r = if t <= 66.0 {
            255.0
        } else {
            329.698_73 * (t - 60.0).powf(-0.133_204_76)
        };
        let g = if t <= 66.0 {
            99.470_8 * t.ln() - 161.119_57
        } else {
            288.122_17 * (t - 60.0).powf(-0.075_514_85)
        };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_73 * (t - 10.0).ln() - 305.044_8
        };
        [r, g, b].map(|c| c.clamp(0.0, 255.0) / 255.0)
    }
    // A 6500 K la fórmula no da el blanco exacto (verde 0,997 y azul 0,980):
    // sin normalizar, «sin teñir» ya amarillearía un poco.
    let base = crudo(f64::from(LuzNocturna::NEUTRA));
    let c = crudo(kelvin);
    [0, 1, 2].map(|i| (c[i] / base[i]).clamp(0.0, 1.0))
}

/// Una rampa de gamma de `n` entradas para un canal: la identidad escalada.
// 0.1.0
pub fn rampa(n: usize, multiplicador: f32) -> Vec<u16> {
    let ultimo = n.saturating_sub(1).max(1) as f32;
    (0..n)
        .map(|i| (i as f32 / ultimo * multiplicador * 65535.0).round() as u16)
        .collect()
}

/// Lee la configuración y va a lo que toque. Se llama al arrancar y al recargar
/// los ajustes. Cambiar la configuración desde Ajustes descarta lo forzado a
/// mano: quien acaba de elegir un modo quiere ver ese modo.
// 0.1.0
pub fn recargar(state: &mut BookosComp, config: LuzNocturna) {
    state.luz_nocturna.config = config;
    state.luz_nocturna.forzada = None;
    ir_al_objetivo(state);
    programar(state);
}

/// Encender o apagar desde la baldosa o la tarjeta de brillo.
// 0.1.0
pub fn alternar(state: &mut BookosComp, encender: bool) {
    let luz = &mut state.luz_nocturna;
    match luz.config.modo {
        // Con horario se fuerza hasta el próximo borde; ver la cabecera.
        ModoLuzNocturna::Programada => luz.forzada = Some(encender),
        ModoLuzNocturna::Apagada | ModoLuzNocturna::Encendida => {
            let nuevo = if encender {
                ModoLuzNocturna::Encendida
            } else {
                ModoLuzNocturna::Apagada
            };
            if luz.config.modo != nuevo {
                luz.config.modo = nuevo;
                if let Err(err) = bookos_shell::guardar_luz_nocturna(nuevo) {
                    tracing::warn!("no se pudo guardar la luz nocturna: {err}");
                }
            }
        }
    }
    ir_al_objetivo(state);
}

/// Vuelve a poner la curva que había al despertar de otro TTY o reconectar una
/// pantalla, evitando el destello blanco antes del primer fotograma compuesto.
// 0.1.0
pub fn reponer(state: &mut BookosComp) {
    let luz = &mut state.luz_nocturna;
    if (luz.aplicada - f64::from(LuzNocturna::NEUTRA)).abs() >= 0.5 || luz.transicion.is_some() {
        luz.pendiente = Some(multiplicadores_f64(luz.aplicada));
        state.needs_redraw = true;
    }
}

/// Avanza un paso del fundido en cada fotograma mientras haya transición activa.
// añadido en la 0.2.4
pub fn avanzar(state: &mut BookosComp) {
    state.luz_nocturna.avanzar_en(Instant::now());
}

impl Estado {
    fn avanzar_en(&mut self, ahora: Instant) {
        let Some(transicion) = self.transicion.as_ref() else {
            return;
        };
        if bookos_shell::tema::efectos_reducidos() {
            let objetivo = transicion.objetivo_kelvin;
            self.aplicada = f64::from(objetivo);
            self.pendiente = Some(multiplicadores(objetivo));
            self.transicion = None;
            return;
        }
        let kelvin = transicion.kelvin(ahora);
        let terminada = transicion.terminada(ahora);
        self.aplicada = kelvin;
        self.pendiente = Some(multiplicadores_f64(kelvin));
        if terminada {
            self.transicion = None;
        }
    }
}

/// Funde de lo que hay a lo que toca ahora.
// 0.1.0
// modificado en la 0.2.4 -- muelle continuo en mireds sincronizado con el bucle VSync
fn ir_al_objetivo(state: &mut BookosComp) {
    let ahora_hora = bookos_shell::hora_local_ahora();
    let objetivo = state.luz_nocturna.objetivo(ahora_hora);
    bookos_shell::marcar_luz_nocturna(state.luz_nocturna.encendida(ahora_hora));
    // Las tarjetas abiertas enseñan el estado: que se repinten.
    state.needs_redraw = true;
    let luz = &mut state.luz_nocturna;
    // Sin animaciones no hay fundido: el cambio entero en un paso.
    if bookos_shell::tema::efectos_reducidos() {
        luz.transicion = None;
        luz.aplicada = f64::from(objetivo);
        luz.pendiente = Some(multiplicadores(objetivo));
        return;
    }
    let ahora = Instant::now();
    if let Some(transicion) = luz.transicion.as_mut() {
        transicion.dirigir(objetivo, ahora);
        if transicion.terminada(ahora) {
            luz.transicion = None;
            luz.aplicada = f64::from(objetivo);
            luz.pendiente = Some(multiplicadores(objetivo));
        }
        return;
    }
    if (luz.aplicada - f64::from(objetivo)).abs() >= 0.5 {
        luz.transicion = Some(Transicion::nueva(luz.aplicada, objetivo, ahora));
    }
}

/// Programa el despertar del próximo borde del horario, o lo quita.
// 0.1.0
fn programar(state: &mut BookosComp) {
    if let Some(token) = state.luz_nocturna.tick.take() {
        state.loop_handle.remove(token);
    }
    let Some(minutos) = state
        .luz_nocturna
        .config
        .minutos_al_cambio(bookos_shell::hora_local_ahora())
    else {
        return;
    };
    let falta = std::time::Duration::from_secs(minutos as u64 * 60).max(MINIMO);
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(falta), |_, _, state| {
            state.luz_nocturna.tick = None;
            // En el borde el horario vuelve a mandar.
            state.luz_nocturna.forzada = None;
            ir_al_objetivo(state);
            programar(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(token) => state.luz_nocturna.tick = Some(token),
        Err(err) => tracing::error!("no se pudo programar la luz nocturna: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_6500_no_tine_nada() {
        assert_eq!(multiplicadores(LuzNocturna::NEUTRA), [1.0, 1.0, 1.0]);
    }

    /// Cálida quita azul y algo de verde, nunca rojo; fría quita rojo.
    #[test]
    fn calida_quita_azul_y_fria_quita_rojo() {
        let [r, g, b] = multiplicadores(4000);
        assert_eq!(r, 1.0);
        assert!(g < 1.0 && b < g, "a 4000 K: {g} {b}");
        let [r, g, b] = multiplicadores(LuzNocturna::MAXIMA);
        assert!(r < g && g < 1.0 && b == 1.0, "a 8000 K: {r} {g} {b}");
    }

    /// Más cálida, menos azul: el deslizador tiene que ir siempre en el mismo
    /// sentido o el usuario no sabe hacia dónde moverlo.
    #[test]
    fn el_azul_baja_con_la_temperatura() {
        let azules: Vec<f32> = (LuzNocturna::MINIMA..=LuzNocturna::NEUTRA)
            .step_by(100)
            .map(|k| multiplicadores(k)[2])
            .collect();
        assert!(azules.windows(2).all(|p| p[0] <= p[1]), "{azules:?}");
        assert!(azules[0] > 0.0, "a 2500 K queda algo de azul");
    }

    #[test]
    fn la_rampa_va_de_cero_al_multiplicador() {
        let r = rampa(256, 1.0);
        assert_eq!((r[0], r[255]), (0, 65535));
        let r = rampa(1024, 0.5);
        assert_eq!(r[1023], 32768);
        assert!(r.windows(2).all(|p| p[0] <= p[1]));
    }

    #[test]
    fn la_escala_en_mireds_es_perceptualmente_uniforme() {
        let m_calido = a_mired(2500.0);
        let m_neutro = a_mired(6500.0);
        let medio = desde_mired((m_calido + m_neutro) * 0.5);
        assert!(
            (medio - 3611.11).abs() < 1.0,
            "punto medio en mireds debe caer cerca de 3611 K, dio {medio}"
        );
    }

    #[test]
    fn el_muelle_calienta_y_enfria_sin_saltos_ni_sobrepaso() {
        use std::time::Duration;
        let ahora = Instant::now();
        let duracion = Duration::from_secs(4);
        for (desde, objetivo) in [(6500.0, 2700), (2700.0, 6500), (4500.0, 3500)] {
            let t = Transicion::con_efectos(desde, 0.0, objetivo, ahora, true);
            assert!((t.kelvin(ahora) - desde).abs() < 1e-6);
            let muestras: Vec<f64> = (0..=100)
                .map(|i| t.kelvin(ahora + Duration::from_millis(1600) * i / 100))
                .collect();
            let fin = f64::from(objetivo);
            assert!(
                muestras
                    .iter()
                    .all(|k| (desde.min(fin)..=desde.max(fin)).contains(k)),
                "no debe rebasar {desde} -> {fin}"
            );
            assert!(muestras.windows(2).all(|p| if desde < fin {
                p[0] <= p[1] + 1e-9
            } else {
                p[0] >= p[1] - 1e-9
            }));
            let max_paso = muestras
                .windows(2)
                .map(|p| (p[1] - p[0]).abs())
                .fold(0.0_f64, f64::max);
            assert!((muestras[1] - desde).abs() < max_paso);
            assert!((fin - muestras[99]).abs() < max_paso);
            assert!(t.terminada(ahora + duracion));
            assert_eq!(t.kelvin(ahora + duracion), fin);
        }
    }

    #[test]
    fn invertir_sentido_a_mitad_del_fundido_conserva_temperatura() {
        use std::time::Duration;
        let ahora = Instant::now();
        let mut t = Transicion::con_efectos(6500.0, 0.0, 2700, ahora, true);
        let cambio = ahora + Duration::from_millis(250);
        let antes = t.kelvin(cambio);
        assert!(antes < 6500.0 && antes > 2700.0);
        t.dirigir(6500, cambio);
        assert!((t.kelvin(cambio) - antes).abs() < 1e-6);
        assert!(t.kelvin(cambio + Duration::from_millis(16)) > antes);
        assert_eq!(t.kelvin(cambio + Duration::from_secs(4)), 6500.0);
    }

    #[test]
    fn redirigir_en_mismo_sentido_conserva_velocidad_sin_sobrepaso() {
        use std::time::Duration;
        let ahora = Instant::now();
        let mut t = Transicion::con_efectos(6500.0, 0.0, 3500, ahora, true);
        let cambio = ahora + Duration::from_millis(220);
        let (mired_antes, vel_antes) = t.estado(cambio);
        assert!(vel_antes > 0.0);
        t.dirigir(2500, cambio);
        let (mired_despues, vel_despues) = t.estado(cambio);
        assert!((mired_despues - mired_antes).abs() < 1e-9);
        assert!((vel_despues - vel_antes).abs() < 1e-9);
        assert!((0..=100).all(|i| {
            let k = t.kelvin(cambio + Duration::from_millis(20) * i);
            (2500.0..=6500.0).contains(&k)
        }));
    }

    #[test]
    fn volver_a_neutro_es_mas_agil_que_calentar() {
        use std::time::Duration;
        let ahora = Instant::now();
        let calentar = Transicion::con_efectos(6500.0, 0.0, 3000, ahora, true);
        let enfriar = Transicion::con_efectos(3000.0, 0.0, 6500, ahora, true);
        let instante = ahora + Duration::from_millis(250);
        let (m_cal, _) = calentar.estado(instante);
        let (m_enf, _) = enfriar.estado(instante);
        let frac_cal = (m_cal - a_mired(6500.0)) / (a_mired(3000.0) - a_mired(6500.0));
        let frac_enf = (a_mired(3000.0) - m_enf) / (a_mired(3000.0) - a_mired(6500.0));
        assert!(frac_enf > frac_cal);
    }

    #[test]
    fn sin_efectos_salta_en_un_solo_paso() {
        let ahora = Instant::now();
        let t = Transicion::con_efectos(6500.0, 0.0, 3000, ahora, false);
        assert!(t.terminada(ahora));
        assert_eq!(t.kelvin(ahora), 3000.0);
    }

    #[test]
    fn efectos_reducidos_en_medio_del_fundido_remata_transicion_de_golpe() {
        struct Restaurar(bool);
        impl Drop for Restaurar {
            fn drop(&mut self) {
                bookos_shell::tema::aplicar_efectos_reducidos(self.0);
            }
        }
        let _guard = Restaurar(bookos_shell::tema::efectos_reducidos());
        bookos_shell::tema::aplicar_efectos_reducidos(false);

        let ahora = Instant::now();
        let mut estado = Estado::nuevo(LuzNocturna::default());
        estado.transicion = Some(Transicion::nueva(6500.0, 3200, ahora));
        assert!(estado.animando());

        bookos_shell::tema::aplicar_efectos_reducidos(true);
        estado.avanzar_en(ahora);
        assert!(!estado.animando());
        assert_eq!(estado.aplicada, 3200.0);
        assert_eq!(estado.tomar_pendiente(), Some(multiplicadores(3200)));
    }
}
