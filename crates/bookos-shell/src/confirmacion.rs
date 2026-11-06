//! Confirmación compartida por el escritorio y la pantalla de bloqueo.
use crate::{TeclaPulsada, tema, view::PanelElement};
use iced_core::{Border, Font, Length, font::Weight};
use iced_widget::{Space, column, container, row, text};

/// ¿Puede hibernar este equipo? Lo decide logind (`CanHibernate`): hace falta
/// swap en disco del tamaño de la RAM y `resume=` en el arranque, y sin eso
/// `systemctl hibernate` falla. Con zram como única swap, que es lo que trae
/// Fedora, contesta `na`, así que la opción no se enseña.
///
/// Se pregunta una vez por proceso (3-4 ms medidos con `busctl`). Lo que cambia
/// la respuesta —swap nueva, `resume=`— necesita reiniciar de todos modos.
// 0.1.0
pub fn hibernar_disponible() -> bool {
    static DISPONIBLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DISPONIBLE.get_or_init(|| {
        std::process::Command::new("busctl")
            .args([
                "--timeout=2s",
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                "CanHibernate",
            ])
            .output()
            .is_ok_and(|salida| String::from_utf8_lossy(&salida.stdout).trim() == "s \"yes\"")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Energia {
    Suspender,
    Hibernar,
    Bloquear,
    CerrarSesion,
    Reiniciar,
    Apagar,
}

impl Energia {
    // 0.1.0
    pub fn etiqueta(self) -> &'static str {
        match self {
            Self::Suspender => "Suspender",
            Self::Hibernar => "Hibernar",
            Self::Bloquear => "Bloquear",
            Self::CerrarSesion => "Cerrar sesión",
            Self::Reiniciar => "Reiniciar",
            Self::Apagar => "Apagar",
        }
    }
    // 0.1.0
    pub fn de_accion(accion: &crate::Accion) -> Option<Self> {
        match accion {
            // Bloquear no se confirma: no se pierde nada y se deshace con la
            // contraseña. Preguntarlo convertía Meta+L —que en cualquier
            // sistema bloquea al momento— en un diálogo, y encima con Intro
            // en «Cancelar».
            crate::Accion::CerrarSesion => Some(Self::CerrarSesion),
            crate::Accion::Lanzar(cmd) => match cmd.as_str() {
                "systemctl suspend" => Some(Self::Suspender),
                "systemctl hibernate" => Some(Self::Hibernar),
                "systemctl reboot" => Some(Self::Reiniciar),
                "systemctl poweroff" => Some(Self::Apagar),
                _ => None,
            },
            _ => None,
        }
    }
}

pub struct Confirmacion {
    pub accion: Energia,
    aceptar: bool,
}
impl Confirmacion {
    pub const ANCHO: f32 = 310.0;
    pub const ALTO: f32 = 208.0;
    // 0.1.0
    pub fn new(accion: Energia) -> Self {
        Self {
            accion,
            aceptar: false,
        }
    }
    /// None: seguir abierto; false: cancelar; true: ejecutar.
    // 0.1.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Option<bool> {
        match tecla {
            TeclaPulsada::Escape => Some(false),
            TeclaPulsada::Izquierda => {
                self.aceptar = false;
                None
            }
            TeclaPulsada::Derecha => {
                self.aceptar = true;
                None
            }
            TeclaPulsada::Tabulador => {
                self.aceptar = !self.aceptar;
                None
            }
            TeclaPulsada::Intro => Some(self.aceptar),
            _ => None,
        }
    }
    // 0.1.0
    pub fn pulsar(&self, x: f32, y: f32) -> Option<bool> {
        if !(140.0..188.0).contains(&y) {
            return None;
        }
        if (24.0..150.0).contains(&x) {
            Some(false)
        } else if (160.0..286.0).contains(&x) {
            Some(true)
        } else {
            None
        }
    }
    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let mensaje = match self.accion {
            Energia::Bloquear => "Necesitarás autenticarte para volver a entrar.",
            Energia::Suspender => "El equipo entrará en reposo. Tu sesión se conservará.",
            Energia::Hibernar => {
                "Tu sesión se guardará en el disco y el equipo se apagará. Al encenderlo volverás donde estabas."
            }
            _ => "Guarda tu trabajo antes de continuar. Se cerrarán las aplicaciones abiertas.",
        };
        let boton = |label, primario, foco| {
            let fondo = if primario {
                tema::acento()
            } else {
                tema::alfa(tema::tinta(), 0.06)
            };
            container(
                text(label)
                    .size(15)
                    .font(Font {
                        weight: Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(if primario {
                        tema::tinta_sobre(fondo)
                    } else {
                        tema::texto()
                    }),
            )
            .center_x(Length::Fixed(126.0))
            .center_y(Length::Fixed(48.0))
            .style(move |_| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: 14.0.into(),
                    width: if foco { 2.0 } else { 0.0 },
                    color: tema::texto(),
                },
                ..Default::default()
            })
        };
        let contenido = column![
            container(
                text(format!("¿{}?", self.accion.etiqueta()))
                    .size(17)
                    .font(Font {
                        weight: Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto())
            )
            .center_x(Length::Fill)
            .height(24),
            Space::new().height(12),
            container(
                text(mensaje)
                    .size(13)
                    .color(tema::TEXTO2)
                    .align_x(iced_core::alignment::Horizontal::Center)
            )
            .center_x(Length::Fill)
            .height(56),
            Space::new().height(20),
            row![
                boton("Cancelar", false, !self.aceptar),
                Space::new().width(10),
                boton(self.accion.etiqueta(), true, self.aceptar)
            ],
        ];
        container(contenido)
            .padding(iced_core::Padding {
                top: 28.0,
                right: 24.0,
                bottom: 20.0,
                left: 24.0,
            })
            .width(Self::ANCHO)
            .height(Self::ALTO)
            .style(|_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: 26.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intro_cancela_por_defecto() {
        let mut c = Confirmacion::new(Energia::Apagar);
        assert_eq!(c.tecla(TeclaPulsada::Intro), Some(false));
        assert_eq!(c.tecla(TeclaPulsada::Derecha), None);
        assert_eq!(c.tecla(TeclaPulsada::Intro), Some(true));
        assert_eq!(c.tecla(TeclaPulsada::Escape), Some(false));
    }
    #[test]
    fn botones_y_huecos_coinciden() {
        let c = Confirmacion::new(Energia::Reiniciar);
        assert_eq!(c.pulsar(80.0, 164.0), Some(false));
        assert_eq!(c.pulsar(210.0, 164.0), Some(true));
        assert_eq!(c.pulsar(155.0, 164.0), None);
        assert_eq!(c.pulsar(210.0, 120.0), None);
    }
}
