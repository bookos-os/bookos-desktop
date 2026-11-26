//! Reconocimiento de gestos sin Smithay para probarlos con tiempos deterministas.

use std::collections::VecDeque;

// añadido en la 0.2.0 -- el eje queda elegido hasta levantar la mano para evitar dos acciones diagonales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movimiento {
    Escritorio,
    Exposicion,
    Launchpad,
    // añadido en la 0.2.0 -- cuatro dedos en vertical apartan o devuelven las ventanas siguiendo la mano.
    Despeje,
    // añadido en la 0.2.0 -- cuatro dedos hacia arriba abren la vista de escritorios (Meta+W) siguiendo la mano; hacia abajo la cierran.
    Vista,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesto {
    Continuo(Movimiento, f32),
    DespejarEscritorio,
    SubirCuatroDedos,
}

// añadido en la 0.2.0 -- posiciones normalizadas; la velocidad se expresa en recorridos por segundo.
#[derive(Debug, Clone, Copy)]
pub struct Suelta {
    pub movimiento: Movimiento,
    pub progreso: f32,
    pub velocidad: f32,
    pub cancelada: bool,
}

impl Suelta {
    // modificado en la 0.2.0 -- el impulso pesa por su recorrido, no por cruzar una velocidad arbitraria.
    pub fn destino(&self) -> f32 {
        if self.cancelada || !self.progreso.is_finite() {
            0.0
        } else {
            let velocidad = if self.velocidad.is_finite() {
                self.velocidad
            } else {
                0.0
            };
            bookos_shell::tema::proyectar_impulso(self.progreso, velocidad)
                .clamp(-1.0, 1.0)
                .round()
        }
    }
}

// añadido en la 0.2.0 -- conserva el agarre real y descarta el recorrido sobrante en los límites.
#[derive(Debug, Clone, Copy)]
pub struct Agarre {
    pub posicion: f32,
    destino_anterior: f32,
    ultimo: f32,
}

impl Agarre {
    // añadido en la 0.2.0
    pub fn nuevo(posicion: f32, destino_anterior: f32) -> Self {
        Self {
            posicion,
            destino_anterior,
            ultimo: 0.0,
        }
    }

    // añadido en la 0.2.0 -- invertir en un extremo mueve desde el primer incremento contrario.
    pub fn seguir(&mut self, progreso: f32, minimo: f32, maximo: f32) -> f32 {
        if !progreso.is_finite() {
            return self.posicion;
        }
        self.posicion = (self.posicion + progreso - self.ultimo).clamp(minimo, maximo);
        self.ultimo = progreso;
        self.posicion
    }

    /// Sigue el gesto permitiendo un sobrepaso elástico amortiguado fuera de `[minimo, maximo]`
    /// (`holgura_min` por debajo, `holgura_max` por encima; HIG §8) y retorno inmediato 1:1
    /// al invertir la dirección. Con movimiento reducido activo (o holgura `<= 0`) recorta al límite.
    pub fn seguir_con_goma(
        &mut self,
        progreso: f32,
        minimo: f32,
        maximo: f32,
        holgura_min: f32,
        holgura_max: f32,
    ) -> f32 {
        if !progreso.is_finite() {
            return self.posicion;
        }
        let delta = progreso - self.ultimo;
        self.ultimo = progreso;
        if bookos_shell::tema::efectos_reducidos() {
            self.posicion = (self.posicion + delta).clamp(minimo, maximo);
            return self.posicion;
        }
        let lineal = self.posicion + delta;
        self.posicion = if delta > 0.0 && lineal > maximo {
            if holgura_max <= 0.0 {
                maximo
            } else {
                let r_prev = if self.posicion > maximo {
                    desde_goma(self.posicion - maximo, holgura_max)
                } else {
                    0.0
                };
                let exceso = if self.posicion > maximo {
                    r_prev + delta
                } else {
                    lineal - maximo
                };
                maximo + hacia_goma(exceso, holgura_max)
            }
        } else if delta < 0.0 && lineal < minimo {
            if holgura_min <= 0.0 {
                minimo
            } else {
                let r_prev = if self.posicion < minimo {
                    desde_goma(minimo - self.posicion, holgura_min)
                } else {
                    0.0
                };
                let exceso = if self.posicion < minimo {
                    r_prev - delta
                } else {
                    minimo - lineal
                };
                minimo - hacia_goma(exceso, holgura_min)
            }
        } else {
            lineal
        };
        self.posicion
    }

    // añadido en la 0.2.0 -- cancelar una animación agarrada recupera su destino, no una mitad arbitraria.
    pub fn destino(&self, suelta: Suelta) -> f32 {
        if suelta.cancelada {
            self.destino_anterior
        } else {
            Suelta {
                progreso: self.posicion,
                ..suelta
            }
            .destino()
        }
    }
}

const RESISTENCIA_GOMA: f32 = 0.55;

fn hacia_goma(exceso: f32, holgura: f32) -> f32 {
    if exceso <= 0.0 || holgura <= 0.0 {
        return 0.0;
    }
    (1.0 - 1.0 / (exceso * RESISTENCIA_GOMA / holgura + 1.0)) * holgura
}

fn desde_goma(desplazamiento: f32, holgura: f32) -> f32 {
    if desplazamiento <= 0.0 || holgura <= 0.0 {
        return 0.0;
    }
    let acotado = desplazamiento.min(holgura * 0.999);
    (holgura / RESISTENCIA_GOMA) * (acotado / (holgura - acotado))
}

/// Dedos del pellizco del launchpad. Solo cinco: con cuatro, separar un poco
/// la mano al deslizar entre escritorios lo convertía en pellizco.
// modificado en la 0.2.0 -- antes 4..=5
const DEDOS: std::ops::RangeInclusive<u32> = 5..=5;
// Con 5 dedos juntos, reducir el radio medio un 32 % y abrirlo un 55 % resulta
// simétrico al alcance natural de la mano antes de que los dedos choquen.
const CERRAR: f64 = 0.68;
const ABRIR: f64 = 1.55;
const DESLIZAR: f64 = 120.0;
// añadido en la 0.2.0 -- con 240 la exposición llegaba al tope con un tercio
// de la mano y un gesto amplio se sentía demasiado rápido. Sin medir en el
// touchpad: es el doble de largo que antes y quizá haya que afinarlo.
const RECORRIDO_VERTICAL: f64 = 480.0;
const DOMINIO: f64 = 2.0;
// añadido en la 0.2.0 -- filtra el temblor del apoyo sin esperar un plazo fijo.
const UMBRAL_DESLIZAMIENTO: f64 = 10.0;

// añadido en la 0.2.0 -- la ventana temporal incluye el fin: parar antes de soltar elimina el impulso.
#[derive(Debug, Default)]
pub(crate) struct Velocidad {
    muestras: VecDeque<(u64, f32)>,
}

impl Velocidad {
    // añadido en la 0.2.0
    pub(crate) fn anotar(&mut self, tiempo: u64, valor: f32) {
        if !valor.is_finite() {
            return;
        }
        let tiempo = self.muestras.back().map_or(tiempo, |(t, _)| tiempo.max(*t));
        self.muestras.push_back((tiempo, valor));
        while self.muestras.len() > 2
            && self
                .muestras
                .get(1)
                .is_some_and(|(t, _)| tiempo - t >= 50_000)
        {
            self.muestras.pop_front();
        }
    }

    pub(crate) fn al_soltar(&mut self, tiempo: u64) -> f32 {
        if let Some(&(_, ultimo)) = self.muestras.back() {
            self.anotar(tiempo, ultimo);
        }
        self.valor()
    }

    // añadido en la 0.2.0
    pub(crate) fn valor(&self) -> f32 {
        match (self.muestras.front(), self.muestras.back()) {
            (Some((inicio, desde)), Some((fin, hasta))) if fin > inicio => {
                (hasta - desde) * 1_000_000.0 / (fin - inicio) as f32
            }
            _ => 0.0,
        }
    }
}

#[derive(Debug)]
struct Deslizamiento {
    dedos: u32,
    x: f64,
    y: f64,
    ancho: f64,
    movimiento: Option<Movimiento>,
    disparado: bool,
    /// Hacia dónde sigue a la mano el despeje con cuatro dedos (+1 abajo, −1
    /// arriba). Lo decide el compositor al apoyar, que sabe si hay ventanas
    /// apartadas; el otro sentido conserva su acción discreta.
    despeje: Option<f64>,
    /// Si la vista de escritorios está abierta al apoyar la mano. `None` deja
    /// a cuatro dedos en vertical con su acción discreta de siempre.
    vista: Option<bool>,
}

#[derive(Debug, Default)]
pub struct Gestos {
    deslizamiento: Option<Deslizamiento>,
    pellizco: Option<u32>,
    progreso: f32,
    velocidad: Velocidad,
}

impl Gestos {
    // modificado en la 0.2.0 -- el ancho se fija al inicio, aunque el puntero cruce de monitor.
    pub fn deslizamiento_inicio(&mut self, dedos: u32, ancho: f64, tiempo: u64) {
        self.deslizamiento = Some(Deslizamiento {
            dedos,
            x: 0.0,
            y: 0.0,
            ancho,
            movimiento: None,
            disparado: false,
            despeje: None,
            vista: None,
        });
        self.progreso = 0.0;
        self.velocidad = Velocidad::default();
        self.velocidad.anotar(tiempo, 0.0);
    }

    // añadido en la 0.2.0
    pub fn despeje_continuo(&mut self, sentido: f64) {
        if let Some(d) = self.deslizamiento.as_mut() {
            d.despeje = Some(sentido);
        }
    }

    // añadido en la 0.2.0
    pub fn vista_continua(&mut self, abierta: bool) {
        if let Some(d) = self.deslizamiento.as_mut() {
            d.vista = Some(abierta);
        }
    }

    // modificado en la 0.2.0 -- reconocer el eje inicia el seguimiento; cruzar un umbral ya no cambia de escritorio.
    pub fn deslizamiento_avance(&mut self, dx: f64, dy: f64, tiempo: u64) -> Option<Gesto> {
        if !dx.is_finite() || !dy.is_finite() {
            return None;
        }
        let d = self.deslizamiento.as_mut()?;
        d.x += dx;
        d.y += dy;
        if d.disparado || !(3..=4).contains(&d.dedos) {
            return None;
        }
        if d.movimiento.is_none() {
            if d.dedos == 4 && d.x.abs() > UMBRAL_DESLIZAMIENTO && d.x.abs() > d.y.abs() * DOMINIO {
                d.movimiento = Some(Movimiento::Escritorio);
            } else if d.y.abs() > UMBRAL_DESLIZAMIENTO && d.y.abs() > d.x.abs() * DOMINIO {
                if d.dedos == 3 {
                    d.movimiento = Some(Movimiento::Exposicion);
                } else if d.despeje.is_some_and(|s| d.y * s > 0.0) {
                    d.movimiento = Some(Movimiento::Despeje);
                // Subir abre la vista y, con ella abierta, bajar la cierra: el
                // otro sentido no sigue a la mano.
                } else if d.vista.is_some_and(|abierta| (d.y < 0.0) != abierta) {
                    d.movimiento = Some(Movimiento::Vista);
                } else if d.y.abs() > DESLIZAR {
                    d.disparado = true;
                    return Some(if d.y > 0.0 {
                        Gesto::DespejarEscritorio
                    } else {
                        Gesto::SubirCuatroDedos
                    });
                }
            }
        }
        let movimiento = d.movimiento?;
        self.progreso = match movimiento {
            Movimiento::Escritorio => (-d.x / d.ancho.max(1.0)) as f32,
            Movimiento::Exposicion => (-d.y / RECORRIDO_VERTICAL) as f32,
            Movimiento::Despeje => (d.y * d.despeje.unwrap_or(1.0) / RECORRIDO_VERTICAL) as f32,
            Movimiento::Vista => (-d.y / RECORRIDO_VERTICAL) as f32,
            Movimiento::Launchpad => unreachable!("un deslizamiento no es un pellizco"),
        };
        self.velocidad.anotar(tiempo, self.progreso);
        Some(Gesto::Continuo(movimiento, self.progreso))
    }

    // modificado en la 0.2.0
    pub fn deslizamiento_fin(&mut self, cancelada: bool, tiempo: u64) -> Option<Suelta> {
        let d = self.deslizamiento.take()?;
        self.velocidad.anotar(tiempo, self.progreso);
        Some(Suelta {
            movimiento: d.movimiento.or(match d.dedos {
                3 => Some(Movimiento::Exposicion),
                4 => Some(Movimiento::Escritorio),
                _ => None,
            })?,
            progreso: self.progreso,
            velocidad: self.velocidad.valor(),
            // El eje no cambia a mitad de gesto, pero una mano que subía con
            // algo de deriva al apoyar quedaba en «escritorio», y soltarla con
            // impulso cambiaba de escritorio sin haberlo pedido. Si al final el
            // recorrido es vertical, la intención era otra: vuelve a su sitio.
            cancelada: cancelada
                || d.movimiento.is_none()
                || (d.movimiento == Some(Movimiento::Escritorio)
                    && d.y.abs() > d.x.abs() * DOMINIO),
        })
    }

    // 0.1.0
    pub fn pellizco_de_la_ventana(&self) -> bool {
        self.pellizco.is_some_and(|dedos| !DEDOS.contains(&dedos))
    }

    // modificado en la 0.2.0
    pub fn pellizco_inicio(&mut self, dedos: u32, tiempo: u64) {
        self.pellizco = Some(dedos);
        self.progreso = 0.0;
        self.velocidad = Velocidad::default();
        self.velocidad.anotar(tiempo, 0.0);
    }

    // modificado en la 0.2.0 -- el signo representa cerrar o abrir; la asimetría conserva el recorrido de libinput medido.
    pub fn pellizco_avance(&mut self, escala: f64, tiempo: u64) -> Option<f32> {
        if !escala.is_finite() || escala <= 0.0 {
            return None;
        }
        if !DEDOS.contains(&self.pellizco?) {
            return None;
        }
        self.progreso = if escala <= 1.0 {
            ((1.0 - escala) / (1.0 - CERRAR)) as f32
        } else {
            ((1.0 - escala) / (ABRIR - 1.0)) as f32
        };
        self.velocidad.anotar(tiempo, self.progreso);
        Some(self.progreso)
    }

    // modificado en la 0.2.0
    pub fn pellizco_fin(&mut self, cancelada: bool, tiempo: u64) -> Option<Suelta> {
        let dedos = self.pellizco.take()?;
        if !DEDOS.contains(&dedos) {
            return None;
        }
        self.velocidad.anotar(tiempo, self.progreso);
        Some(Suelta {
            movimiento: Movimiento::Launchpad,
            progreso: self.progreso,
            velocidad: self.velocidad.valor(),
            cancelada,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // añadido en la 0.2.0
    #[test]
    fn una_correccion_cerca_del_destino_no_devuelve_todo_el_recorrido() {
        let fin = Suelta {
            movimiento: Movimiento::Escritorio,
            progreso: 0.9,
            velocidad: -1.2,
            cancelada: false,
        };
        assert_eq!(fin.destino(), 1.0);
        assert_eq!(
            Suelta {
                velocidad: -3.0,
                ..fin
            }
            .destino(),
            0.0
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn los_extremos_no_acumulan_recorrido_invisible() {
        let mut agarre = Agarre::nuevo(0.5, 1.0);
        assert_eq!(agarre.seguir(2.0, 0.0, 1.0), 1.0);
        assert!((agarre.seguir(1.9, 0.0, 1.0) - 0.9).abs() < 0.0001);
        assert_eq!(agarre.seguir(-2.0, 0.0, 1.0), 0.0);
        assert!((agarre.seguir(-1.9, 0.0, 1.0) - 0.1).abs() < 0.0001);
    }

    // añadido en la 0.2.0
    #[test]
    fn cancelar_una_apertura_agarrada_por_debajo_de_la_mitad_la_completa() {
        let mut agarre = Agarre::nuevo(0.2, 1.0);
        agarre.seguir(-0.1, 0.0, 1.0);
        assert_eq!(
            agarre.destino(Suelta {
                movimiento: Movimiento::Exposicion,
                progreso: -0.1,
                velocidad: -2.0,
                cancelada: true
            }),
            1.0
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn reconocer_el_deslizamiento_exige_diez_puntos_sin_esperar_un_plazo() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        assert_eq!(g.deslizamiento_avance(-9.0, 0.0, 1), None);
        assert_eq!(
            g.deslizamiento_avance(-2.0, 0.0, 2),
            Some(Gesto::Continuo(Movimiento::Escritorio, 0.011))
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn apoyar_y_soltar_sin_elegir_eje_libera_la_animacion_agarrada() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        let fin = g
            .deslizamiento_fin(false, 1)
            .expect("cuatro dedos pueden haber agarrado una tira");
        assert!(fin.cancelada);
        assert_eq!(fin.movimiento, Movimiento::Escritorio);
    }

    // añadido en la 0.2.0
    #[test]
    fn soltar_pasada_la_mitad_cambia() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.deslizamiento_avance(-600.0, 0.0, 1_000_000);
        let fin = g
            .deslizamiento_fin(false, 1_200_000)
            .expect("gesto horizontal reconocido");
        assert_eq!(fin.destino(), 1.0);
        assert_eq!(fin.progreso, 0.6);
    }

    // añadido en la 0.2.0
    #[test]
    fn un_golpe_rapido_y_corto_cambia() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.deslizamiento_avance(-100.0, 0.0, 30_000);
        assert_eq!(
            g.deslizamiento_fin(false, 35_000)
                .expect("gesto reconocido")
                .destino(),
            1.0
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn soltar_lento_sin_pasar_vuelve() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.deslizamiento_avance(-400.0, 0.0, 1_000_000);
        assert_eq!(
            g.deslizamiento_fin(false, 1_200_000)
                .expect("gesto reconocido")
                .destino(),
            0.0
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn parar_antes_de_soltar_descarta_el_impulso() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        for i in 1..=5 {
            g.deslizamiento_avance(-20.0, 0.0, i * 7_000);
        }
        assert_eq!(
            g.deslizamiento_fin(false, 200_000)
                .expect("gesto reconocido")
                .destino(),
            0.0
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn cancelar_siempre_vuelve() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.deslizamiento_avance(-900.0, 0.0, 30_000);
        assert_eq!(
            g.deslizamiento_fin(true, 35_000)
                .expect("gesto reconocido")
                .destino(),
            0.0
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn los_ejes_no_cambian_a_mitad_de_gesto() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.deslizamiento_avance(20.0, 0.0, 10_000);
        assert!(matches!(
            g.deslizamiento_avance(0.0, 300.0, 20_000),
            Some(Gesto::Continuo(Movimiento::Escritorio, _))
        ));
        assert!(
            g.deslizamiento_fin(false, 30_000)
                .expect("gesto reconocido")
                .progreso
                < 0.0
        );
    }

    #[test]
    fn subir_con_deriva_al_apoyar_no_cambia_de_escritorio() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        // Deriva lateral antes de que la mano arranque: fija el eje horizontal.
        assert!(matches!(
            g.deslizamiento_avance(-300.0, 0.0, 10),
            Some(Gesto::Continuo(Movimiento::Escritorio, _))
        ));
        g.deslizamiento_avance(0.0, -700.0, 20);
        let suelta = g.deslizamiento_fin(false, 21).expect("gesto reconocido");
        assert_eq!(suelta.destino(), 0.0, "subía: no debe cambiar");
    }

    // añadido en la 0.2.0
    #[test]
    fn tres_dedos_exponen_y_cuatro_despejan_una_vez() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(3, 1000.0, 0);
        assert_eq!(
            g.deslizamiento_avance(0.0, -120.0, 100_000),
            Some(Gesto::Continuo(Movimiento::Exposicion, 0.25))
        );
        g.deslizamiento_fin(false, 120_000);
        g.deslizamiento_inicio(4, 1000.0, 200_000);
        assert_eq!(
            g.deslizamiento_avance(0.0, 130.0, 300_000),
            Some(Gesto::DespejarEscritorio)
        );
        assert_eq!(g.deslizamiento_avance(0.0, 130.0, 310_000), None);
    }

    // añadido en la 0.2.0
    #[test]
    fn cuatro_dedos_siguen_al_despeje_solo_en_su_sentido() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.despeje_continuo(1.0);
        assert_eq!(
            g.deslizamiento_avance(0.0, 240.0, 100_000),
            Some(Gesto::Continuo(Movimiento::Despeje, 0.5))
        );
        let suelta = g
            .deslizamiento_fin(false, 120_000)
            .expect("gesto reconocido");
        assert_eq!(suelta.movimiento, Movimiento::Despeje);

        // Despejando hacia abajo, subir sigue siendo la acción de siempre.
        g.deslizamiento_inicio(4, 1000.0, 200_000);
        g.despeje_continuo(1.0);
        assert_eq!(
            g.deslizamiento_avance(0.0, -130.0, 300_000),
            Some(Gesto::SubirCuatroDedos)
        );

        // Devolviendo, subir es avanzar.
        g.deslizamiento_inicio(4, 1000.0, 400_000);
        g.despeje_continuo(-1.0);
        assert_eq!(
            g.deslizamiento_avance(0.0, -240.0, 500_000),
            Some(Gesto::Continuo(Movimiento::Despeje, 0.5))
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn el_pellizco_de_dos_dedos_sigue_siendo_del_cliente() {
        let mut g = Gestos::default();
        g.pellizco_inicio(2, 0);
        assert!(g.pellizco_de_la_ventana());
        assert_eq!(g.pellizco_avance(0.3, 10_000), None);
        assert!(g.pellizco_fin(false, 20_000).is_none());
        assert!(!g.pellizco_de_la_ventana());
        g.pellizco_inicio(4, 0);
        assert!(
            g.pellizco_de_la_ventana(),
            "con cuatro dedos ya no abre el launchpad"
        );
        assert_eq!(g.pellizco_avance(0.7, 10_000), None);
        assert!(g.pellizco_fin(false, 20_000).is_none());
        assert!(!g.pellizco_de_la_ventana());
        g.pellizco_inicio(5, 0);
        assert_eq!(g.pellizco_avance(CERRAR, 100_000), Some(1.0));
        assert_eq!(g.pellizco_avance(ABRIR, 200_000), Some(-1.0));
    }

    // añadido en la 0.2.0
    #[test]
    fn cuatro_dedos_hacia_arriba_siguen_a_la_mano_con_la_vista_cerrada() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.vista_continua(false);
        let Some(Gesto::Continuo(Movimiento::Vista, p)) = g.deslizamiento_avance(0.0, -48.0, 1)
        else {
            panic!("subir con la vista cerrada debe seguir a la mano");
        };
        assert!((p - 0.1).abs() < 1e-6);
        // Y sigue creciendo con la mano, sin esperar a ningún umbral.
        let Some(Gesto::Continuo(Movimiento::Vista, p)) = g.deslizamiento_avance(0.0, -96.0, 2)
        else {
            panic!("el seguimiento continúa");
        };
        assert!((p - 0.3).abs() < 1e-6);
    }

    // añadido en la 0.2.0
    #[test]
    fn con_la_vista_abierta_bajar_la_cierra_siguiendo_la_mano() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.vista_continua(true);
        let Some(Gesto::Continuo(Movimiento::Vista, p)) = g.deslizamiento_avance(0.0, 48.0, 1)
        else {
            panic!("bajar con la vista abierta debe seguir a la mano");
        };
        // Negativo: el progreso va hacia cerrar.
        assert!((p + 0.1).abs() < 1e-6);
    }

    /// Sin saber si la vista está abierta —o con el escritorio apartado, que
    /// manda el despeje— los cuatro dedos hacia arriba conservan lo de antes.
    #[test]
    fn el_despeje_sigue_mandando_cuando_las_ventanas_estan_apartadas() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        g.despeje_continuo(-1.0);
        g.vista_continua(false);
        assert!(matches!(
            g.deslizamiento_avance(0.0, -48.0, 1),
            Some(Gesto::Continuo(Movimiento::Despeje, _))
        ));

        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        assert_eq!(g.deslizamiento_avance(0.0, -48.0, 1), None);
        assert_eq!(
            g.deslizamiento_avance(0.0, -100.0, 2),
            Some(Gesto::SubirCuatroDedos)
        );
    }

    #[test]
    fn la_goma_frena_el_sobrepaso_y_devuelve_al_invertir() {
        let mut a = Agarre::nuevo(0.0, 0.0);
        // Sube dentro del rango 1:1.
        assert!((a.seguir_con_goma(0.6, 0.0, 1.0, 0.0, 0.14) - 0.6).abs() < 1e-6);
        // Al pasar de 1.0, avanza menos que el dedo y siempre por debajo de 1.0 + holgura.
        let p1 = a.seguir_con_goma(1.2, 0.0, 1.0, 0.0, 0.14);
        let p2 = a.seguir_con_goma(1.6, 0.0, 1.0, 0.0, 0.14);
        assert!(p1 > 1.0 && p1 < 1.14, "p1={p1}");
        assert!(p2 > p1 && p2 < 1.14, "p2={p2}");
        assert!((p2 - p1) < 0.4, "debe amortiguar el exceso");

        // Invertir 0.02 responde de inmediato sin zona muerta.
        let vuelta = a.seguir_con_goma(1.58, 0.0, 1.0, 0.0, 0.14);
        assert!(vuelta < p2, "vuelta={vuelta} debería bajar respecto a p2={p2}");
        assert!((vuelta - (p2 - 0.02)).abs() < 1e-5);

        // Sin holgura inferior no pasa de 0.0 al empujar en negativo, y sí con holgura > 0.
        let mut sin_min = Agarre::nuevo(0.0, 0.0);
        assert_eq!(sin_min.seguir_con_goma(-0.3, 0.0, 1.0, 0.0, 0.14), 0.0);
        let mut b = Agarre::nuevo(0.0, 0.0);
        let neg = b.seguir_con_goma(-0.3, 0.0, 1.0, 0.14, 0.14);
        assert!(neg < 0.0 && neg > -0.14, "neg={neg}");
    }

    #[test]
    fn ancho_cero_y_progreso_no_finito_no_producen_nan() {
        let mut a = Agarre::nuevo(0.4, 0.0);
        assert_eq!(a.seguir(f32::NAN, 0.0, 1.0), 0.4);
        assert_eq!(a.seguir_con_goma(f32::INFINITY, 0.0, 1.0, 0.1, 0.1), 0.4);

        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 0.0, 0);
        let Some(Gesto::Continuo(Movimiento::Escritorio, p)) = g.deslizamiento_avance(-20.0, 0.0, 1)
        else {
            panic!("esperado movimiento de escritorio");
        };
        assert!(p.is_finite());
    }

    #[test]
    fn deltas_y_sueltas_no_finitos_o_escala_no_positiva_no_corrompen_gestos() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4, 1000.0, 0);
        assert_eq!(g.deslizamiento_avance(f64::NAN, 0.0, 5), None);
        assert_eq!(g.deslizamiento_avance(0.0, f64::INFINITY, 6), None);
        assert!(matches!(
            g.deslizamiento_avance(-200.0, 0.0, 10_000),
            Some(Gesto::Continuo(Movimiento::Escritorio, p)) if p.is_finite()
        ));

        g.pellizco_inicio(5, 20_000);
        assert_eq!(g.pellizco_avance(0.0, 21_000), None);
        assert_eq!(g.pellizco_avance(-1.0, 22_000), None);
        assert_eq!(g.pellizco_avance(f64::NAN, 23_000), None);

        assert_eq!(
            Suelta {
                movimiento: Movimiento::Escritorio,
                progreso: f32::NAN,
                velocidad: 1.0,
                cancelada: false,
            }
            .destino(),
            0.0
        );
        assert_eq!(
            Suelta {
                movimiento: Movimiento::Escritorio,
                progreso: 0.8,
                velocidad: f32::INFINITY,
                cancelada: false,
            }
            .destino(),
            1.0
        );
    }
}
