//! Máquina de estados de navegación táctil estilo Android/iOS para `bookos-extras`.
//!
//! Sigue el HIG de BookOS (§8) y `03-gesture-physics.md`:
//! - Seguimiento 1:1 del dedo con goma en los extremos y cero retardo doble.
//! - Borde superior en 2 pasos (como Android): 1º revela el panel, 2º abre acción.
//! - Borde inferior hacia arriba en 2 pasos (como iOS):
//!   1º la app/ventanas siguen el dedo 1:1 hasta quedar en medio (exposición);
//!   2º desde la exposición, volver a deslizar de abajo hacia arriba manda al escritorio.
//! - Borde izquierdo = Atrás (`XF86Back`), borde derecho = Adelante (`XF86Forward`),
//!   con cápsula flotante fluida que sigue al dedo en X e Y.

use crate::config::Config;
use crate::fisica::{VentanaVelocidad, goma, proyectar_impulso};
use crate::protocolo::{Borde, Peticion};

const ZONA_IZQUIERDA: f64 = 0.28;
const ZONA_DERECHA: f64 = 0.72;
const UMBRAL_ARRIBA_DISCRETO: f64 = 56.0;
const RECORRIDO_CORTINA: f64 = 300.0;
const RECORRIDO_ATRAS: f64 = 104.0;
const UMBRAL_ATRAS_PROGRESO: f32 = 0.50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lado {
    Izquierda,
    Derecha,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EfectoLocal {
    CortinaProgreso(f32),
    CortinaSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    IndicadorFlotante {
        lado: Lado,
        y: f64,
        desplazamiento_x: f32,
        progreso: f32,
        armado: bool,
    },
    IndicadorSoltar {
        disparado: bool,
    },
}

#[derive(Debug, Default)]
pub struct SalidaGesto {
    pub peticiones: Vec<Peticion>,
    pub locales: Vec<EfectoLocal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modo {
    ArribaMostrarPanel { disparado: bool },
    ArribaIzquierda { disparado: bool },
    ArribaCentro,
    ArribaDerecha { disparado: bool },
    AbajoIndeciso { exposicion_abierta: bool },
    AbajoVertical { exposicion_abierta: bool },
    AbajoHorizontal { iniciado: bool },
    NavegacionLateral(Lado),
}

#[derive(Debug, Clone)]
struct GestoActivo {
    slot: u32,
    origen_x: f64,
    origen_y: f64,
    ultimo_x: f64,
    ultimo_y: f64,
    vel_x: VentanaVelocidad,
    vel_y: VentanaVelocidad,
    ancho: f64,
    alto: f64,
    modo: Modo,
}

#[derive(Debug, Default)]
pub struct MotorGestos {
    activo: Option<GestoActivo>,
}

impl MotorGestos {
    #[allow(clippy::too_many_arguments)]
    pub fn inicio(
        &mut self,
        cfg: &Config,
        slot: u32,
        x: f64,
        y: f64,
        ancho: f64,
        alto: f64,
        borde: Borde,
        panel_visible: bool,
        exposicion_abierta: bool,
        tiempo: u32,
    ) -> SalidaGesto {
        let mut salida = self.cancelar_todo();
        if !cfg.activo
            || !x.is_finite()
            || !y.is_finite()
            || !ancho.is_finite()
            || !alto.is_finite()
            || ancho <= 0.0
            || alto <= 0.0
        {
            return salida;
        }
        let modo = match borde {
            Borde::Arriba => {
                if !panel_visible {
                    Modo::ArribaMostrarPanel { disparado: false }
                } else {
                    let frac = (x / ancho).clamp(0.0, 1.0);
                    if frac < ZONA_IZQUIERDA {
                        Modo::ArribaIzquierda { disparado: false }
                    } else if frac > ZONA_DERECHA {
                        Modo::ArribaDerecha { disparado: false }
                    } else {
                        salida.peticiones.push(Peticion::CortinaAbierta);
                        salida.peticiones.push(Peticion::PedirNotificaciones);
                        salida.locales.push(EfectoLocal::CortinaProgreso(0.02));
                        Modo::ArribaCentro
                    }
                }
            }
            Borde::Abajo => {
                if cfg.gesto_inferior_horizontal {
                    Modo::AbajoIndeciso { exposicion_abierta }
                } else {
                    Modo::AbajoVertical { exposicion_abierta }
                }
            }
            Borde::Izquierda => {
                if !cfg.gesto_atras {
                    return salida;
                }
                Modo::NavegacionLateral(Lado::Izquierda)
            }
            Borde::Derecha => {
                if !cfg.gesto_atras {
                    return salida;
                }
                Modo::NavegacionLateral(Lado::Derecha)
            }
        };

        let mut vel_x = VentanaVelocidad::default();
        let mut vel_y = VentanaVelocidad::default();
        vel_x.reiniciar(tiempo, 0.0);
        vel_y.reiniciar(tiempo, 0.0);

        self.activo = Some(GestoActivo {
            slot,
            origen_x: x,
            origen_y: y,
            ultimo_x: x,
            ultimo_y: y,
            vel_x,
            vel_y,
            ancho,
            alto,
            modo,
        });
        salida
    }

    pub fn mueve(&mut self, slot: u32, x: f64, y: f64, tiempo: u32) -> SalidaGesto {
        let mut salida = SalidaGesto::default();
        if !x.is_finite() || !y.is_finite() {
            return salida;
        }
        let Some(gesto) = self.activo.as_mut().filter(|g| g.slot == slot) else {
            return salida;
        };

        gesto.ultimo_x = x;
        gesto.ultimo_y = y;

        let dx = x - gesto.origen_x;
        let dy = y - gesto.origen_y;

        match &mut gesto.modo {
            Modo::ArribaMostrarPanel { disparado } => {
                if !*disparado && dy >= 24.0 {
                    *disparado = true;
                    salida.peticiones.push(Peticion::MostrarPanel);
                }
            }
            Modo::ArribaIzquierda { disparado } => {
                if !*disparado && dy >= UMBRAL_ARRIBA_DISCRETO {
                    *disparado = true;
                    salida.peticiones.push(Peticion::MenuBookos);
                }
            }
            Modo::ArribaDerecha { disparado } => {
                if !*disparado && dy >= UMBRAL_ARRIBA_DISCRETO {
                    *disparado = true;
                    salida.peticiones.push(Peticion::CentroControl);
                }
            }
            Modo::ArribaCentro => {
                let raw = (dy.max(0.0) / RECORRIDO_CORTINA) as f32;
                let progreso = if raw <= 1.0 {
                    raw
                } else {
                    1.0 + goma(raw - 1.0, 0.12)
                };
                gesto.vel_y.anotar(tiempo, raw);
                salida.locales.push(EfectoLocal::CortinaProgreso(progreso));
            }
            Modo::AbajoIndeciso { exposicion_abierta } => {
                // `bookos-comp` ya filtró los primeros 10 px: decidimos eje al instante (latencia cero).
                let abierta = *exposicion_abierta;
                if dx.abs() > (-dy).max(0.0) * 1.15 {
                    gesto.modo = Modo::AbajoHorizontal { iniciado: true };
                    let p = (-dx / (gesto.ancho * 0.45)).clamp(-1.0, 1.0) as f32;
                    gesto.vel_x.anotar(tiempo, p);
                    salida.peticiones.push(Peticion::EscritorioInicio);
                    salida
                        .peticiones
                        .push(Peticion::EscritorioProgreso { progreso: p });
                } else {
                    gesto.modo = Modo::AbajoVertical {
                        exposicion_abierta: abierta,
                    };
                    let subida = (-dy).max(0.0);
                    let desplazamiento_x =
                        (dx / (gesto.ancho * 0.45)).clamp(-1.0, 1.0) as f32;
                    if abierta {
                        let progreso = (subida / (gesto.alto * 0.42)).clamp(0.0, 1.0) as f32;
                        gesto.vel_y.anotar(tiempo, progreso);
                        salida.peticiones.push(Peticion::ExposicionEscritorioProgreso {
                            progreso,
                            desplazamiento_x,
                        });
                    } else {
                        let progreso = (subida / (gesto.alto * 0.38)).clamp(0.0, 1.0) as f32;
                        gesto.vel_y.anotar(tiempo, progreso);
                        salida.peticiones.push(Peticion::ExposicionProgreso {
                            progreso,
                            desplazamiento_x,
                        });
                    }
                }
            }
            Modo::AbajoVertical { exposicion_abierta } => {
                let subida = (-dy).max(0.0);
                let desplazamiento_x =
                    (dx / (gesto.ancho * 0.45)).clamp(-1.0, 1.0) as f32;
                if *exposicion_abierta {
                    let progreso = (subida / (gesto.alto * 0.42)).clamp(0.0, 1.0) as f32;
                    gesto.vel_y.anotar(tiempo, progreso);
                    salida.peticiones.push(Peticion::ExposicionEscritorioProgreso {
                        progreso,
                        desplazamiento_x,
                    });
                } else {
                    let progreso = (subida / (gesto.alto * 0.38)).clamp(0.0, 1.0) as f32;
                    gesto.vel_y.anotar(tiempo, progreso);
                    salida.peticiones.push(Peticion::ExposicionProgreso {
                        progreso,
                        desplazamiento_x,
                    });
                }
            }
            Modo::AbajoHorizontal { iniciado } => {
                if !*iniciado {
                    *iniciado = true;
                    salida.peticiones.push(Peticion::EscritorioInicio);
                }
                let progreso = (-dx / (gesto.ancho * 0.45)).clamp(-1.0, 1.0) as f32;
                gesto.vel_x.anotar(tiempo, progreso);
                salida
                    .peticiones
                    .push(Peticion::EscritorioProgreso { progreso });
            }
            Modo::NavegacionLateral(lado) => {
                let avance = match lado {
                    Lado::Izquierda => dx.max(0.0),
                    Lado::Derecha => (-dx).max(0.0),
                };
                let raw = (avance / RECORRIDO_ATRAS) as f32;
                gesto.vel_x.anotar(tiempo, raw);
                let progreso = raw.clamp(0.0, 1.0);
                let desplazamiento_x = if avance <= RECORRIDO_ATRAS {
                    (avance as f32 / RECORRIDO_ATRAS as f32) * 34.0
                } else {
                    34.0 + goma((avance - RECORRIDO_ATRAS) as f32, 18.0)
                };
                let armado = progreso >= UMBRAL_ATRAS_PROGRESO;
                salida.locales.push(EfectoLocal::IndicadorFlotante {
                    lado: *lado,
                    y,
                    desplazamiento_x,
                    progreso,
                    armado,
                });
            }
        }
        salida
    }

    pub fn cancelar_todo(&mut self) -> SalidaGesto {
        let Some(slot) = self.activo.as_ref().map(|g| g.slot) else {
            return SalidaGesto::default();
        };
        self.fin(slot, 0, true)
    }

    pub fn fin(&mut self, slot: u32, tiempo: u32, cancelada: bool) -> SalidaGesto {
        let mut salida = SalidaGesto::default();
        let Some(mut gesto) = self.activo.take() else {
            return salida;
        };
        if gesto.slot != slot {
            self.activo = Some(gesto);
            return salida;
        }

        let dx = gesto.ultimo_x - gesto.origen_x;
        let dy = gesto.ultimo_y - gesto.origen_y;

        match gesto.modo {
            Modo::ArribaMostrarPanel { .. }
            | Modo::ArribaIzquierda { .. }
            | Modo::ArribaDerecha { .. }
            | Modo::AbajoIndeciso { .. } => {}
            Modo::ArribaCentro => {
                let raw = (dy.max(0.0) / RECORRIDO_CORTINA) as f32;
                let vel = if cancelada {
                    0.0
                } else {
                    gesto.vel_y.al_soltar(tiempo)
                };
                let proyectado = proyectar_impulso(raw, vel);
                let abrir = !cancelada && proyectado >= 0.38;
                if abrir {
                    salida.peticiones.push(Peticion::MarcarNotificacionesLeidas);
                }
                salida.locales.push(EfectoLocal::CortinaSoltar {
                    progreso: raw.clamp(0.0, 1.12),
                    velocidad: vel,
                    cancelada: cancelada || !abrir,
                });
            }
            Modo::AbajoVertical { exposicion_abierta } => {
                let subida = (-dy).max(0.0);
                let vel = if cancelada {
                    0.0
                } else {
                    gesto.vel_y.al_soltar(tiempo)
                };
                if exposicion_abierta {
                    // 2º gesto (desde exposición en medio): subir manda al escritorio con inercia.
                    let progreso = (subida / (gesto.alto * 0.42)).clamp(0.0, 1.0) as f32;
                    let proyectado = proyectar_impulso(progreso, vel);
                    let cancelar = cancelada || (progreso < 0.18 && proyectado < 0.35);
                    salida
                        .peticiones
                        .push(Peticion::ExposicionEscritorioSoltar {
                            progreso,
                            velocidad: vel,
                            cancelada: cancelar,
                        });
                } else {
                    // 1º gesto (desde app): sigue el dedo 1:1 hasta quedar en medio (exposición).
                    let progreso = (subida / (gesto.alto * 0.38)).clamp(0.0, 1.0) as f32;
                    let proyectado = proyectar_impulso(progreso, vel);
                    let cancelar = cancelada || (progreso < 0.20 && proyectado < 0.35);
                    salida.peticiones.push(Peticion::ExposicionSoltar {
                        progreso,
                        velocidad: vel,
                        cancelada: cancelar,
                    });
                }
            }
            Modo::AbajoHorizontal { iniciado } => {
                if iniciado {
                    let progreso = (-dx / (gesto.ancho * 0.45)).clamp(-1.0, 1.0) as f32;
                    let velocidad = if cancelada {
                        0.0
                    } else {
                        gesto.vel_x.al_soltar(tiempo)
                    };
                    salida.peticiones.push(Peticion::EscritorioSoltar {
                        progreso,
                        velocidad,
                        cancelada,
                    });
                }
            }
            Modo::NavegacionLateral(lado) => {
                let avance = match lado {
                    Lado::Izquierda => dx.max(0.0),
                    Lado::Derecha => (-dx).max(0.0),
                };
                let raw = (avance / RECORRIDO_ATRAS) as f32;
                let vel = if cancelada {
                    0.0
                } else {
                    gesto.vel_x.al_soltar(tiempo)
                };
                let proyectado = proyectar_impulso(raw, vel);
                let disparado = !cancelada && (raw >= UMBRAL_ATRAS_PROGRESO || proyectado >= 0.68);
                if disparado {
                    match lado {
                        Lado::Izquierda => salida.peticiones.push(Peticion::Atras),
                        Lado::Derecha => salida.peticiones.push(Peticion::Adelante),
                    }
                }
                salida
                    .locales
                    .push(EfectoLocal::IndicadorSoltar { disparado });
            }
        }
        salida
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arriba_dos_pasos_primero_muestra_panel_segundo_abre_accion() {
        let cfg = Config::default();
        let mut motor = MotorGestos::default();

        let _ = motor.inicio(&cfg, 0, 640.0, 4.0, 1280.0, 800.0, Borde::Arriba, false, false, 0);
        let m1 = motor.mueve(0, 640.0, 44.0, 30);
        assert!(m1.peticiones.iter().any(|p| matches!(p, Peticion::MostrarPanel)));
        let _ = motor.fin(0, 40, false);

        let s0 = motor.inicio(&cfg, 1, 640.0, 4.0, 1280.0, 800.0, Borde::Arriba, true, false, 100);
        assert!(
            s0.peticiones
                .iter()
                .any(|p| matches!(p, Peticion::PedirNotificaciones))
        );
        let s1 = motor.mueve(1, 640.0, 180.0, 160);
        assert!(
            s1.locales
                .iter()
                .any(|e| matches!(e, EfectoLocal::CortinaProgreso(p) if *p > 0.5))
        );
        let s2 = motor.fin(1, 170, false);
        assert!(s2.locales.iter().any(|e| matches!(
            e,
            EfectoLocal::CortinaSoltar {
                cancelada: false,
                ..
            }
        )));
    }

    #[test]
    fn abajo_en_dos_pasos_estilo_ios_exposicion_y_luego_escritorio() {
        let cfg = Config::default();
        let mut motor = MotorGestos::default();

        // Paso 1: desde app, arrastrar hacia arriba sigue el dedo 1:1 y deja la app en medio (exposición).
        let _ = motor.inicio(&cfg, 0, 640.0, 795.0, 1280.0, 800.0, Borde::Abajo, true, false, 0);
        let m1 = motor.mueve(0, 640.0, 600.0, 200);
        assert!(
            m1.peticiones
                .iter()
                .any(|p| matches!(p, Peticion::ExposicionProgreso { .. }))
        );
        let f1 = motor.fin(0, 210, false);
        assert!(f1.peticiones.iter().any(|p| matches!(
            p,
            Peticion::ExposicionSoltar {
                cancelada: false,
                ..
            }
        )));

        // Paso 2: ya en medio (exposicion_abierta = true), volver a subir manda al escritorio.
        let _ = motor.inicio(&cfg, 1, 640.0, 795.0, 1280.0, 800.0, Borde::Abajo, true, true, 500);
        let m2 = motor.mueve(1, 640.0, 500.0, 620);
        assert!(
            m2.peticiones
                .iter()
                .any(|p| matches!(p, Peticion::ExposicionEscritorioProgreso { .. }))
        );
        let f2 = motor.fin(1, 630, false);
        assert!(f2.peticiones.iter().any(|p| matches!(
            p,
            Peticion::ExposicionEscritorioSoltar {
                cancelada: false,
                ..
            }
        )));
    }

    #[test]
    fn abajo_horizontal_cambia_escritorio() {
        let cfg = Config::default();
        let mut motor = MotorGestos::default();
        let _ = motor.inicio(&cfg, 0, 640.0, 795.0, 1280.0, 800.0, Borde::Abajo, true, false, 0);
        let m = motor.mueve(0, 820.0, 790.0, 100);
        assert!(
            m.peticiones
                .iter()
                .any(|p| matches!(p, Peticion::EscritorioProgreso { progreso } if *progreso < 0.0))
        );
        let f = motor.fin(0, 110, false);
        assert!(
            f.peticiones
                .iter()
                .any(|p| matches!(p, Peticion::EscritorioSoltar { progreso, .. } if *progreso < 0.0))
        );

        let _ = motor.inicio(&cfg, 1, 640.0, 795.0, 1280.0, 800.0, Borde::Abajo, true, false, 200);
        let m2 = motor.mueve(1, 460.0, 790.0, 260);
        assert!(
            m2.peticiones
                .iter()
                .any(|p| matches!(p, Peticion::EscritorioProgreso { progreso } if *progreso > 0.0))
        );
    }

    #[test]
    fn lateral_izquierdo_atras_y_derecho_adelante() {
        let cfg = Config::default();
        let mut motor = MotorGestos::default();

        let _ = motor.inicio(&cfg, 0, 4.0, 400.0, 1280.0, 800.0, Borde::Izquierda, true, false, 0);
        let m = motor.mueve(0, 88.0, 408.0, 60);
        assert!(m.locales.iter().any(|e| matches!(
            e,
            EfectoLocal::IndicadorFlotante {
                lado: Lado::Izquierda,
                armado: true,
                ..
            }
        )));
        let f = motor.fin(0, 70, false);
        assert!(f.peticiones.iter().any(|p| matches!(p, Peticion::Atras)));

        let _ = motor.inicio(&cfg, 1, 1276.0, 400.0, 1280.0, 800.0, Borde::Derecha, true, false, 200);
        let m2 = motor.mueve(1, 1190.0, 405.0, 260);
        assert!(m2.locales.iter().any(|e| matches!(
            e,
            EfectoLocal::IndicadorFlotante {
                lado: Lado::Derecha,
                armado: true,
                ..
            }
        )));
        let f2 = motor.fin(1, 270, false);
        assert!(f2.peticiones.iter().any(|p| matches!(p, Peticion::Adelante)));
    }

    #[test]
    fn pausar_antes_de_soltar_anula_impulso_y_desconexion_cancela() {
        let cfg = Config::default();
        let mut motor = MotorGestos::default();

        // Tirón corto lateral que habría disparado por inercia, pero el dedo se detiene 200 ms.
        let _ = motor.inicio(&cfg, 0, 4.0, 400.0, 1280.0, 800.0, Borde::Izquierda, true, false, 0);
        let _ = motor.mueve(0, 40.0, 400.0, 20);
        let f = motor.fin(0, 220, false);
        assert!(!f.peticiones.iter().any(|p| matches!(p, Peticion::Atras)));
        assert!(f.locales.iter().any(|e| matches!(e, EfectoLocal::IndicadorSoltar { disparado: false })));

        // Desconexión cancela el gesto activo limpiamente.
        let _ = motor.inicio(&cfg, 1, 640.0, 795.0, 1280.0, 800.0, Borde::Abajo, true, false, 300);
        let _ = motor.mueve(1, 500.0, 790.0, 340);
        let cancel = motor.cancelar_todo();
        assert!(cancel.peticiones.iter().any(|p| matches!(p, Peticion::EscritorioSoltar { cancelada: true, .. })));
    }

    #[test]
    fn coordenadas_no_finitas_se_ignoran_y_nuevo_inicio_cancela_gesto_previo() {
        let cfg = Config::default();
        let mut motor = MotorGestos::default();

        let invalido = motor.inicio(
            &cfg,
            0,
            f64::NAN,
            0.0,
            f64::NAN,
            800.0,
            Borde::Arriba,
            true,
            false,
            0,
        );
        assert!(invalido.peticiones.is_empty() && invalido.locales.is_empty());

        let _ = motor.inicio(&cfg, 1, 4.0, 400.0, 1280.0, 800.0, Borde::Izquierda, true, false, 10);
        assert!(
            motor
                .mueve(1, f64::NAN, 400.0, 20)
                .locales
                .is_empty()
        );
        let nuevo = motor.inicio(&cfg, 2, 640.0, 4.0, 1280.0, 800.0, Borde::Arriba, true, false, 30);
        assert!(
            nuevo
                .locales
                .iter()
                .any(|e| matches!(e, EfectoLocal::IndicadorSoltar { disparado: false })),
            "iniciar un gesto nuevo debe cancelar el anterior"
        );
        assert!(
            nuevo
                .peticiones
                .iter()
                .any(|p| matches!(p, Peticion::PedirNotificaciones))
        );
    }
}
