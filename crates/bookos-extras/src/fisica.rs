//! Física de movimiento del HIG de BookOS (§8) para `bookos-extras`.
//!
//! - Muelles analíticos/semi-implícitos con los tokens oficiales:
//!   - `M_RAPIDO` (amort. 1.0, resp. 0.21 s)
//!   - `M_SUAVE`  (amort. 1.0, resp. 0.315 s)
//!   - `M_REBOTE` (amort. 0.8, resp. 0.28 s — solo con impulso)
//! - Proyección de impulso: `pos + vel * 0.2`
//! - Goma elástica en bordes: `(1 - 1 / (exceso * 0.55 / holgura + 1)) * holgura`
//! - Ventana deslizante de 50–100 ms para estimar velocidad sin ruido.

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct ParametrosMuelle {
    pub amortiguacion: f32,
    pub respuesta: f32,
}

pub const M_RAPIDO: ParametrosMuelle = ParametrosMuelle {
    amortiguacion: 1.0,
    respuesta: 0.21,
};

pub const M_SUAVE: ParametrosMuelle = ParametrosMuelle {
    amortiguacion: 1.0,
    respuesta: 0.315,
};

pub const M_REBOTE: ParametrosMuelle = ParametrosMuelle {
    amortiguacion: 0.8,
    respuesta: 0.28,
};

const RESISTENCIA_GOMA: f32 = 0.55;
const EPS_POS: f32 = 0.0015;
const EPS_VEL: f32 = 0.005;

/// Proyección de impulso del HIG §8 (`posición + velocidad × 0,2`).
pub fn proyectar_impulso(posicion: f32, velocidad: f32) -> f32 {
    if !posicion.is_finite() {
        return 0.0;
    }
    let velocidad = if velocidad.is_finite() { velocidad } else { 0.0 };
    posicion + velocidad * 0.2
}

/// Curva de goma elástica más allá de un borde (`03-gesture-physics.md` §5).
pub fn goma(exceso: f32, holgura: f32) -> f32 {
    if !exceso.is_finite() || !holgura.is_finite() || exceso <= 0.0 || holgura <= 0.0 {
        return 0.0;
    }
    (1.0 - 1.0 / (exceso * RESISTENCIA_GOMA / holgura + 1.0)) * holgura
}

/// Muelle interrumpible: conserva posición y velocidad al redirigir.
#[derive(Debug, Clone)]
pub struct Muelle {
    valor: f32,
    velocidad: f32,
    destino: f32,
    params: ParametrosMuelle,
    ultimo: Instant,
    activo: bool,
}

impl Muelle {
    pub fn nuevo(inicial: f32, params: ParametrosMuelle) -> Self {
        let inicial = if inicial.is_finite() { inicial } else { 0.0 };
        Self {
            valor: inicial,
            velocidad: 0.0,
            destino: inicial,
            params,
            ultimo: Instant::now(),
            activo: false,
        }
    }

    pub fn valor(&self) -> f32 {
        self.valor
    }

    pub fn animando(&self) -> bool {
        self.activo
    }

    /// Fija la posición 1:1 bajo el dedo (interrumpe cualquier muelle en curso).
    pub fn fijar(&mut self, valor: f32) {
        if !valor.is_finite() {
            return;
        }
        self.valor = valor;
        self.destino = valor;
        self.velocidad = 0.0;
        self.activo = false;
        self.ultimo = Instant::now();
    }

    /// Lanza el muelle hacia `destino` conservando o inyectando velocidad inicial.
    pub fn lanzar(
        &mut self,
        destino: f32,
        velocidad_inicial: Option<f32>,
        params: ParametrosMuelle,
        efectos_reducidos: bool,
    ) {
        if !destino.is_finite() {
            return;
        }
        let velocidad_inicial = velocidad_inicial.filter(|v| v.is_finite());
        if efectos_reducidos {
            self.fijar(destino);
            return;
        }
        self.avanzar();
        self.destino = destino;
        if let Some(v) = velocidad_inicial {
            self.velocidad = v;
        }
        self.params = params;
        self.ultimo = Instant::now();
        self.activo = (self.valor - self.destino).abs() > EPS_POS || self.velocidad.abs() > EPS_VEL;
    }

    /// Integra el muelle hasta `Instant::now()`. Devuelve `true` si sigue en marcha.
    pub fn avanzar(&mut self) -> bool {
        if !self.activo {
            return false;
        }
        let ahora = Instant::now();
        let dt_total = ahora.saturating_duration_since(self.ultimo).as_secs_f32().min(0.05);
        self.ultimo = ahora;
        self.integrar_dt(dt_total)
    }

    pub fn integrar_dt(&mut self, dt_total: f32) -> bool {
        if !self.activo || !dt_total.is_finite() || dt_total <= 0.0 {
            return self.activo;
        }
        let omega = TAU / self.params.respuesta.max(0.05);
        let zeta = self.params.amortiguacion.max(0.2);
        // Sub-pasos de ≤ 4 ms para estabilidad numérica total incluso a 60 Hz.
        let pasos = ((dt_total / 0.004).ceil() as usize).clamp(1, 16);
        let h = dt_total / pasos as f32;
        for _ in 0..pasos {
            let x = self.valor - self.destino;
            let a = -2.0 * zeta * omega * self.velocidad - omega * omega * x;
            self.velocidad += a * h;
            self.valor += self.velocidad * h;
        }
        if (self.valor - self.destino).abs() <= EPS_POS && self.velocidad.abs() <= EPS_VEL {
            self.valor = self.destino;
            self.velocidad = 0.0;
            self.activo = false;
        }
        self.activo
    }
}

/// Estimador de velocidad por ventana deslizante de 60–90 ms (evita picos de ruido).
#[derive(Debug, Default, Clone)]
pub struct VentanaVelocidad {
    muestras: VecDeque<(u32, f32)>,
}

impl VentanaVelocidad {
    pub fn reiniciar(&mut self, tiempo_ms: u32, valor: f32) {
        if !valor.is_finite() {
            return;
        }
        self.muestras.clear();
        self.muestras.push_back((tiempo_ms, valor));
    }

    pub fn anotar(&mut self, tiempo_ms: u32, valor: f32) {
        if !valor.is_finite() {
            return;
        }
        let t = self
            .muestras
            .back()
            .map_or(tiempo_ms, |(prev, _)| tiempo_ms.max(*prev));
        self.muestras.push_back((t, valor));
        while self.muestras.len() > 2
            && self
                .muestras
                .get(1)
                .is_some_and(|(t1, _)| t.saturating_sub(*t1) >= 75)
        {
            self.muestras.pop_front();
        }
    }

    /// Añade la muestra final sin movimiento al levantar el dedo para que
    /// pausar antes de soltar descarte el impulso previo.
    pub fn al_soltar(&mut self, tiempo_ms: u32) -> f32 {
        if let Some(&(_, ultimo)) = self.muestras.back() {
            self.anotar(tiempo_ms, ultimo);
        }
        self.valor()
    }

    /// Velocidad en unidades por segundo.
    pub fn valor(&self) -> f32 {
        match (self.muestras.front(), self.muestras.back()) {
            (Some((t0, v0)), Some((t1, v1))) if *t1 > *t0 => {
                (v1 - v0) * 1000.0 / (*t1 - *t0) as f32
            }
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muelle_converge_sin_quedarse_activo() {
        let mut m = Muelle::nuevo(0.0, M_SUAVE);
        m.lanzar(1.0, Some(1.5), M_SUAVE, false);
        assert!(m.animando());
        for _ in 0..80 {
            m.integrar_dt(0.016);
        }
        assert!(!m.animando());
        assert!((m.valor() - 1.0).abs() < 0.002);
    }

    #[test]
    fn ventana_velocidad_filtra_correctamente() {
        let mut vv = VentanaVelocidad::default();
        vv.reiniciar(0, 0.0);
        vv.anotar(25, 0.25);
        vv.anotar(50, 0.50);
        assert!((vv.valor() - 10.0).abs() < 0.1);
    }

    #[test]
    fn pausar_antes_de_soltar_descarta_impulso() {
        let mut vv = VentanaVelocidad::default();
        vv.reiniciar(0, 0.0);
        vv.anotar(20, 0.4);
        vv.anotar(40, 0.8);
        assert!(vv.valor() > 5.0);
        assert_eq!(vv.al_soltar(200), 0.0);
    }

    #[test]
    fn goma_crece_con_rendimiento_decreciente() {
        let a = goma(20.0, 40.0);
        let b = goma(200.0, 40.0);
        assert!(a > 0.0 && a < b && b < 40.0);
    }

    #[test]
    fn entradas_no_finitas_no_envenenan_muelle_ni_velocidad() {
        assert_eq!(proyectar_impulso(f32::NAN, 1.0), 0.0);
        assert_eq!(proyectar_impulso(0.4, f32::INFINITY), 0.4);
        assert_eq!(goma(f32::INFINITY, 10.0), 0.0);
        assert_eq!(goma(5.0, f32::NAN), 0.0);

        let mut m = Muelle::nuevo(f32::NAN, M_SUAVE);
        assert_eq!(m.valor(), 0.0);
        m.fijar(0.5);
        m.fijar(f32::NAN);
        assert_eq!(m.valor(), 0.5);
        m.lanzar(1.0, Some(f32::NAN), M_SUAVE, false);
        assert!(!m.integrar_dt(f32::NAN) || m.valor().is_finite());
        m.lanzar(f32::INFINITY, None, M_SUAVE, false);
        assert!(m.valor().is_finite());

        let mut vv = VentanaVelocidad::default();
        vv.reiniciar(0, 0.0);
        vv.anotar(20, f32::NAN);
        vv.anotar(40, 0.4);
        assert!(vv.valor().is_finite());
    }
}
