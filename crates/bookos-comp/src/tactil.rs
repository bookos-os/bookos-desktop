use std::time::{Duration, Instant};

use smithay::backend::input::TouchSlot;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Logical, Point};

use crate::gestos::{Movimiento, Suelta, Velocidad};
use crate::shell::Barra;
use crate::state::BookosComp;

const TIEMPO_BARRA: Duration = Duration::from_secs(3);
const UMBRAL: f64 = 10.0;
type Punto = Point<f64, Logical>;

fn pantalla_completa(state: &BookosComp) -> Option<bool> {
    let salida = crate::input::salida_tactil(state)?;
    // El panel y el dock pertenecen a la salida principal.
    (salida.current_location() == (0, 0).into()).then(|| state.hay_pantalla_completa_en(salida))
}

pub(crate) fn puede_revelar_barra(state: &BookosComp, barra: Barra) -> bool {
    let Some(completa) = pantalla_completa(state) else {
        return false;
    };
    state.vista_tactil.is_none()
        && !state.cuentagotas
        && !state.salvapantallas.activo()
        && state.shell.as_ref().is_some_and(|s| {
            !s.esta_bloqueado()
                && !s.hay_captura()
                && !s.barra_visible_tactil(barra, completa)
                && !s.hay_conmutador()
                && !s.emergente_tapa_la_pantalla()
        })
}

pub(crate) fn revelar_barra_si_oculta(state: &mut BookosComp, barra: Barra) -> bool {
    if !puede_revelar_barra(state, barra) {
        return false;
    }
    revelar_barra(state, barra);
    true
}

pub(crate) fn revelar_barra(state: &mut BookosComp, barra: Barra) {
    let Some(completa) = pantalla_completa(state) else {
        return;
    };
    let hasta = Instant::now() + TIEMPO_BARRA;
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    if shell.revelar_barra_tactil(barra, hasta, completa) {
        let resultado =
            state
                .loop_handle
                .insert_source(Timer::from_deadline(hasta), move |ahora, _, state| {
                    let siguiente = state
                        .shell
                        .as_mut()
                        .and_then(|s| s.caducar_barra_tactil(barra, ahora, TIEMPO_BARRA));
                    siguiente.map_or_else(
                        || {
                            state.needs_redraw = true;
                            TimeoutAction::Drop
                        },
                        TimeoutAction::ToInstant,
                    )
                });
        if let Err(err) = resultado {
            tracing::warn!(%err, "no se pudo programar la retirada de la barra táctil");
            shell.caducar_barra_tactil(barra, hasta, TIEMPO_BARRA);
        }
    }
    state.needs_redraw = true;
}

pub(crate) fn renovar_barra(state: &mut BookosComp, punto: Punto) {
    if let Some(shell) = state.shell.as_mut() {
        shell.renovar_barra_tactil(punto.x, punto.y, Instant::now() + TIEMPO_BARRA);
    }
}

/// Se deja pasar uno o dos dedos al cliente. Solo se cancela su secuencia
/// cuando tres dedos ya se han movido juntos en horizontal.
#[derive(Default)]
pub(crate) struct Contactos {
    dedos: Vec<(TouchSlot, Punto, Punto)>,
    escritorio: Option<Deslizamiento>,
    inicio_tiempo: u32,
    consumidos: bool,
}

struct Deslizamiento {
    progreso: f32,
    velocidad: Velocidad,
}

impl Contactos {
    pub fn apoyar(&mut self, slot: TouchSlot, punto: Punto, time: u32) -> bool {
        if let Some(d) = self.dedos.iter_mut().find(|(s, _, _)| *s == slot) {
            d.1 = punto;
            d.2 = punto;
        } else {
            self.dedos.push((slot, punto, punto));
        }
        if self.dedos.len() == 3 && !self.consumidos {
            // El gesto comienza al apoyar el tercer dedo, aunque los otros
            // dos ya estuvieran desplazando contenido dentro de la aplicación.
            for (_, origen, actual) in &mut self.dedos {
                *origen = *actual;
            }
            self.inicio_tiempo = time;
        }
        self.consumidos
    }

    fn actualizar_sin_gesto(&mut self, slot: TouchSlot, punto: Punto) {
        if let Some(d) = self.dedos.iter_mut().find(|(s, _, _)| *s == slot) {
            d.1 = punto;
            d.2 = punto;
        }
    }

    fn mover(
        &mut self,
        slot: TouchSlot,
        punto: Punto,
        ancho: f64,
        time: u32,
    ) -> Option<(bool, f32)> {
        self.dedos.iter_mut().find(|(s, _, _)| *s == slot)?.2 = punto;
        let [(_, a, x), (_, b, y), (_, c, z), ..] = self.dedos.as_slice() else {
            return None;
        };
        let d = ((*x - *a) + (*y - *b) + (*z - *c)).downscale(3.0);
        let inicio = self.escritorio.is_none();
        if inicio
            && (self.consumidos
                || self.dedos.len() != 3
                || d.x.abs() < UMBRAL
                || d.x.abs() < d.y.abs() * 1.25
                || [x.x - a.x, y.x - b.x, z.x - c.x]
                    .iter()
                    .any(|dx| dx * d.x.signum() <= 0.0))
        {
            return None;
        }
        let progreso = (-d.x / (ancho.max(1.0) * 0.38)) as f32;
        let inicio_us = u64::from(self.inicio_tiempo) * 1000;
        let gesto = self.escritorio.get_or_insert_with(|| {
            let mut velocidad = Velocidad::default();
            velocidad.anotar(inicio_us, 0.0);
            Deslizamiento {
                progreso: 0.0,
                velocidad,
            }
        });
        gesto.progreso = progreso;
        gesto.velocidad.anotar(u64::from(time) * 1000, progreso);
        self.consumidos = true;
        Some((inicio, progreso))
    }

    fn soltar(&mut self, slot: TouchSlot, time: u32) -> (bool, Option<Suelta>) {
        let consumido = self.consumidos;
        let nuestro = self.dedos.iter().take(3).any(|(s, _, _)| *s == slot);
        self.dedos.retain(|(s, _, _)| *s != slot);
        if self.dedos.is_empty() {
            self.consumidos = false;
        } else if self.escritorio.is_none() && !self.consumidos && self.dedos.len() == 3 {
            for (_, origen, actual) in &mut self.dedos {
                *origen = *actual;
            }
            self.inicio_tiempo = time;
        }
        let suelta = nuestro.then(|| self.terminar(time, false)).flatten();
        (consumido, suelta)
    }

    fn terminar(&mut self, time: u32, cancelada: bool) -> Option<Suelta> {
        let mut gesto = self.escritorio.take()?;
        Some(Suelta {
            movimiento: Movimiento::Escritorio,
            progreso: gesto.progreso,
            velocidad: if cancelada {
                0.0
            } else {
                gesto.velocidad.al_soltar(u64::from(time) * 1000)
            },
            cancelada,
        })
    }
}

pub(crate) fn mover_escritorio(
    state: &mut BookosComp,
    slot: TouchSlot,
    punto: Punto,
    time: u32,
) -> bool {
    if state.contactos.dedos.len() < 3 {
        return state.contactos.consumidos;
    }
    let habilitado = state.vista_tactil.is_none()
        && !state.modo.kiosko()
        && !state.cuentagotas
        && !state.salvapantallas.activo()
        && !state.shell.as_ref().is_some_and(|s| {
            s.esta_bloqueado() || s.hay_captura() || s.hay_conmutador() || s.hay_emergente()
        });
    if state.contactos.escritorio.is_some() || habilitado {
        let ancho = crate::input::salida_tactil(state)
            .and_then(|o| state.space.output_geometry(o))
            .map_or(1.0, |r| f64::from(r.size.w).max(1.0));
        if let Some((inicio, progreso)) = state.contactos.mover(slot, punto, ancho, time) {
            if inicio {
                crate::input::cancelar_toques_cliente(state, time);
                state.pointer_location = punto;
                crate::escritorios::agarrar_dedos(state);
            }
            crate::escritorios::seguir_dedos(state, progreso);
        }
    } else {
        state.contactos.actualizar_sin_gesto(slot, punto);
    }
    state.contactos.consumidos
}

pub(crate) fn soltar_escritorio(state: &mut BookosComp, slot: TouchSlot, time: u32) -> bool {
    let (consumido, suelta) = state.contactos.soltar(slot, time);
    if let Some(suelta) = suelta {
        crate::escritorios::soltar_dedos(state, suelta);
    }
    consumido
}

pub(crate) fn cancelar_escritorio(state: &mut BookosComp, time: u32) {
    if let Some(suelta) = state.contactos.terminar(time, true) {
        crate::escritorios::soltar_dedos(state, suelta);
    }
    state.contactos = Contactos::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_dedos_pasan_al_cliente_y_tres_cambian_de_escritorio() {
        let mut contactos = Contactos::default();
        let [a, b, c] = [0, 1, 2].map(|i| TouchSlot::from(Some(i)));
        for (slot, x) in [(a, 320.0), (b, 400.0)] {
            assert!(!contactos.apoyar(slot, (x, 400.0).into(), 0));
            assert!(
                contactos
                    .mover(slot, (x - 40.0, 400.0).into(), 1280.0, 50)
                    .is_none()
            );
        }
        assert!(!contactos.apoyar(c, (480.0, 400.0).into(), 90));
        assert!(
            contactos
                .mover(a, (240.0, 400.0).into(), 1280.0, 100)
                .is_none()
        );
        assert!(
            contactos
                .mover(b, (320.0, 400.0).into(), 1280.0, 110)
                .is_none()
        );
        assert!(matches!(
            contactos.mover(c, (440.0, 400.0).into(), 1280.0, 120),
            Some((true, progreso)) if progreso > 0.0
        ));
        let suelta = match contactos.soltar(a, 150) {
            (true, Some(s)) => s,
            otro => panic!("se esperaba suelta al levantar el primero: {otro:?}"),
        };
        assert!(suelta.velocidad > 0.0, "el gesto rápido conserva velocidad inicial");
        assert!(matches!(contactos.soltar(b, 150), (true, None)));
        assert!(matches!(contactos.soltar(c, 150), (true, None)));
        assert!(!contactos.apoyar(a, (320.0, 400.0).into(), 200));
    }

    #[test]
    fn barrido_rapido_de_tres_dedos_en_un_solo_paso_conserva_impulso() {
        let mut contactos = Contactos::default();
        let [a, b, c] = [0, 1, 2].map(|i| TouchSlot::from(Some(i)));
        contactos.apoyar(a, (400.0, 400.0).into(), 100);
        contactos.apoyar(b, (450.0, 400.0).into(), 105);
        contactos.apoyar(c, (500.0, 400.0).into(), 110);
        contactos.mover(a, (360.0, 400.0).into(), 1000.0, 120);
        contactos.mover(b, (410.0, 400.0).into(), 1000.0, 120);
        assert!(contactos.mover(c, (460.0, 400.0).into(), 1000.0, 120).is_some());
        let (_, suelta) = contactos.soltar(a, 125);
        let suelta = suelta.expect("debe emitir suelta");
        assert!(suelta.velocidad > 1.0, "velocidad={}", suelta.velocidad);
    }

    #[test]
    fn apoyar_slot_repetido_actualiza_origen() {
        let mut contactos = Contactos::default();
        let a = TouchSlot::from(Some(0));
        assert!(!contactos.apoyar(a, (100.0, 100.0).into(), 10));
        assert!(!contactos.apoyar(a, (500.0, 500.0).into(), 20));
        assert_eq!(contactos.dedos.len(), 1);
        assert_eq!(contactos.dedos[0].1, Point::from((500.0, 500.0)));
    }

    #[test]
    fn ancho_cero_no_produce_infinito_y_cuatro_a_tres_reancora() {
        let mut contactos = Contactos::default();
        let [a, b, c, d] = [0, 1, 2, 3].map(|i| TouchSlot::from(Some(i)));
        for (slot, x) in [(a, 100.0), (b, 200.0), (c, 300.0), (d, 400.0)] {
            contactos.apoyar(slot, (x, 300.0).into(), 10);
        }
        // Mover los 4 dedos no dispara gesto; al soltar el 4.º se reanclan los 3.
        for (slot, x) in [(a, 160.0), (b, 260.0), (c, 360.0), (d, 460.0)] {
            assert!(contactos.mover(slot, (x, 300.0).into(), 0.0, 20).is_none());
        }
        assert!(matches!(contactos.soltar(d, 25), (false, None)));
        assert_eq!(contactos.dedos.len(), 3);
        // Un desplazamiento minúsculo tras soltar el 4.º no hereda los 60 px previos.
        assert!(contactos.mover(a, (162.0, 300.0).into(), 0.0, 30).is_none());
        // Si los 3 superan el umbral incluso con ancho=0.0, el progreso es finito.
        contactos.mover(a, (140.0, 300.0).into(), 0.0, 40);
        contactos.mover(b, (240.0, 300.0).into(), 0.0, 40);
        let (_, progreso) = contactos
            .mover(c, (340.0, 300.0).into(), 0.0, 40)
            .expect("gesto iniciado");
        assert!(progreso.is_finite());
    }
}
