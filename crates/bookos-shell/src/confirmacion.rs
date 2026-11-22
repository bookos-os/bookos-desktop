//! Confirmación compartida por el escritorio y la pantalla de bloqueo.
use crate::emergente::{FilaAccesible, t};
use crate::icono;
use crate::{TeclaPulsada, tema, view::PanelElement};
use iced_core::alignment::Horizontal;
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
            Self::Suspender => t("Suspender", "Sleep"),
            Self::Hibernar => t("Hibernar", "Hibernate"),
            Self::Bloquear => t("Bloquear", "Lock"),
            Self::CerrarSesion => t("Cerrar sesión", "Log out"),
            Self::Reiniciar => t("Reiniciar", "Restart"),
            Self::Apagar => t("Apagar", "Shut down"),
        }
    }

    /// Título completo del diálogo de confirmación (HIG §9: nombra la acción).
    // añadido en la 0.2.3
    pub fn titulo(self) -> &'static str {
        match self {
            Self::Suspender => t("¿Suspender el equipo?", "Put the computer to sleep?"),
            Self::Hibernar => t("¿Hibernar el equipo?", "Hibernate the computer?"),
            Self::Bloquear => t("¿Bloquear la pantalla?", "Lock the screen?"),
            Self::CerrarSesion => t("¿Cerrar la sesión?", "Log out of this session?"),
            Self::Reiniciar => t("¿Reiniciar el equipo?", "Restart the computer?"),
            Self::Apagar => t("¿Apagar el equipo?", "Shut down the computer?"),
        }
    }

    /// Consecuencias reales y concisas para el cuerpo del diálogo.
    // añadido en la 0.2.3
    pub fn mensaje(self) -> &'static str {
        match self {
            Self::Bloquear => t(
                "Necesitarás autenticarte para volver a entrar.",
                "You will need to authenticate to unlock.",
            ),
            Self::Suspender => t(
                "El equipo entrará en reposo. Tu sesión permanecerá abierta.",
                "The computer will go to sleep. Your session will stay open.",
            ),
            Self::Hibernar => t(
                "Tu sesión se guardará en disco y el equipo se apagará.",
                "Your session will be saved to disk and the computer will turn off.",
            ),
            Self::CerrarSesion => t(
                "Se cerrarán tus apps abiertas y volverás a la pantalla de acceso.",
                "Your open apps will close and you will return to the login screen.",
            ),
            Self::Reiniciar => t(
                "Se cerrarán las aplicaciones y el equipo volverá a iniciarse.",
                "Applications will close and the computer will restart.",
            ),
            Self::Apagar => t(
                "Se cerrarán las aplicaciones abiertas. Guarda los cambios pendientes.",
                "Open applications will close. Save any unsaved changes.",
            ),
        }
    }

    /// Pictograma asociado en `icono::PROPIOS`.
    // añadido en la 0.2.3
    pub fn icono(self) -> &'static str {
        match self {
            Self::Suspender => "dormir",
            Self::Hibernar => "hibernar",
            Self::Bloquear => "bloquear",
            Self::CerrarSesion => "salir",
            Self::Reiniciar => "reiniciar",
            Self::Apagar => "apagar",
        }
    }

    /// Solo las acciones que cierran sesión o apagan el sistema piden
    /// confirmación en el escritorio; suspender y bloquear son reversibles.
    // añadido en la 0.2.3
    pub fn pide_confirmacion(self) -> bool {
        matches!(self, Self::CerrarSesion | Self::Reiniciar | Self::Apagar)
    }

    /// Solo «Apagar» usa el color rojo destructivo; el resto de acciones de
    /// energía (incluido «Reiniciar») usan superficie neutra y el acento del PC.
    // añadido en la 0.2.3
    pub fn es_destructiva(self) -> bool {
        matches!(self, Self::Apagar)
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

const PADDING_TOP: f32 = 16.0;
const PADDING_H: f32 = 24.0;
const PADDING_BOTTOM: f32 = 20.0;
const INSIGNIA: f32 = 40.0;
const ICONO_INSIGNIA: f32 = 20.0;
const GAP_INSIGNIA_TITULO: f32 = 8.0;
const ALTO_TITULO: f32 = 22.0;
const GAP_TITULO_CUERPO: f32 = 6.0;
const LH_CUERPO: f32 = 18.0;
const GAP_ERROR: f32 = 6.0;
const ALTO_ERROR: f32 = 16.0;
const GAP_BOTONES: f32 = 12.0;
const ALTO_BOTON: f32 = 48.0;
const HUECO_BOTONES: f32 = 10.0;
const R_DIALOGO_CIRCULAR: f32 = 34.0;

pub struct Confirmacion {
    pub accion: Energia,
    aceptar: bool,
    ancho: f32,
    lineas: Vec<String>,
    error: Option<String>,
    señalado: Option<bool>,
}

impl Confirmacion {
    pub const ANCHO: f32 = 310.0;
    pub const ANCHO_AMPLIO: f32 = 420.0;
    pub const ALTO: f32 = 208.0;

    // 0.1.0
    pub fn new(accion: Energia) -> Self {
        Self::con_ancho(accion, Self::ANCHO, accion.mensaje())
    }

    /// Variante amplia (420 px del HIG §5) para explicar detalles o apps que bloquean el cierre.
    // añadido en la 0.2.3
    pub fn amplia(accion: Energia, detalle: &str) -> Self {
        let mensaje = if detalle.trim().is_empty() {
            accion.mensaje()
        } else {
            detalle
        };
        Self::con_ancho(accion, Self::ANCHO_AMPLIO, mensaje)
    }

    fn con_ancho(accion: Energia, ancho: f32, mensaje: &str) -> Self {
        let interior = ancho - PADDING_H * 2.0;
        let lineas = crate::escritorio::lineas(mensaje, interior, 13.0, 6);
        Self {
            accion,
            aceptar: false,
            ancho,
            lineas,
            error: None,
            señalado: None,
        }
    }

    /// Muestra un aviso de error real dentro del propio diálogo sin cerrarlo.
    // añadido en la 0.2.3
    pub fn poner_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub fn limpiar_error(&mut self) {
        self.error = None;
    }

    fn alto_cuerpo(&self) -> f32 {
        (self.lineas.len().max(2) as f32) * LH_CUERPO
    }

    fn alto_error(&self) -> f32 {
        if self.error.is_some() {
            GAP_ERROR + ALTO_ERROR
        } else {
            0.0
        }
    }

    pub fn size(&self) -> (f32, f32) {
        let alto = PADDING_TOP
            + INSIGNIA
            + GAP_INSIGNIA_TITULO
            + ALTO_TITULO
            + GAP_TITULO_CUERPO
            + self.alto_cuerpo()
            + self.alto_error()
            + GAP_BOTONES
            + ALTO_BOTON
            + PADDING_BOTTOM;
        (self.ancho, alto)
    }

    fn y_botones(&self) -> f32 {
        self.size().1 - PADDING_BOTTOM - ALTO_BOTON
    }

    fn ancho_boton(&self) -> f32 {
        (self.ancho - PADDING_H * 2.0 - HUECO_BOTONES) / 2.0
    }

    fn boton_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = self.y_botones();
        if !(y0..y0 + ALTO_BOTON).contains(&y) {
            return None;
        }
        let w = self.ancho_boton();
        let x_cancelar_fin = PADDING_H + w;
        let x_aceptar_ini = x_cancelar_fin + HUECO_BOTONES;
        let x_aceptar_fin = x_aceptar_ini + w;
        if (PADDING_H..x_cancelar_fin).contains(&x) {
            Some(false)
        } else if (x_aceptar_ini..x_aceptar_fin).contains(&x) {
            Some(true)
        } else {
            None
        }
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.boton_en(x, y));
        let cambia = nueva != self.señalado;
        self.señalado = nueva;
        cambia
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
            TeclaPulsada::Intro | TeclaPulsada::Caracter(' ') => Some(self.aceptar),
            _ => None,
        }
    }

    // 0.1.0
    pub fn pulsar(&self, x: f32, y: f32) -> Option<bool> {
        self.boton_en(x, y)
    }

    /// Controles expuestos a AT-SPI2 / Orca.
    // añadido en la 0.2.3
    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let y0 = self.y_botones();
        let w = self.ancho_boton();
        vec![
            FilaAccesible {
                id: "confirmar:cancelar".into(),
                rol: accesskit::Role::Button,
                nombre: t("Cancelar", "Cancel").into(),
                valor: String::new(),
                rect: (PADDING_H, y0, w, ALTO_BOTON),
                marcada: None,
                seleccionada: !self.aceptar,
            },
            FilaAccesible {
                id: "confirmar:aceptar".into(),
                rol: accesskit::Role::Button,
                nombre: self.accion.etiqueta().into(),
                valor: String::new(),
                rect: (PADDING_H + w + HUECO_BOTONES, y0, w, ALTO_BOTON),
                marcada: None,
                seleccionada: self.aceptar,
            },
        ]
    }

    // 0.1.0
    // modified 0.2.3 -- insignia semántica, pareja destructiva accesible y contraste HIG
    pub fn view(&self) -> PanelElement<'_> {
        let (_, alto_total) = self.size();
        let ancho_boton = self.ancho_boton();
        let destructiva = self.accion.es_destructiva();

        let (fondo_insignia, tinta_insignia) = if destructiva {
            let fondo = tema::rojo_suave();
            (fondo, tema::rojo_en(fondo))
        } else {
            (tema::superficie_en_tarjeta(), tema::texto())
        };
        let glifo: PanelElement<'_> = match icono::propio_firme(self.accion.icono()) {
            Some(ic) => icono::ver_teñido_propio(&ic, ICONO_INSIGNIA, tinta_insignia),
            None => Space::new().width(ICONO_INSIGNIA).into(),
        };
        let insignia = container(glifo)
            .width(INSIGNIA)
            .height(INSIGNIA)
            .center_x(INSIGNIA)
            .center_y(INSIGNIA)
            .style(move |_| container::Style {
                background: Some(fondo_insignia.into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let (fondo_dest, tinta_dest) = tema::pareja_destructiva();
        let boton = |label: &str, es_aceptar: bool, foco: bool| {
            let (base, tinta) = if !es_aceptar {
                (tema::superficie_en_tarjeta(), tema::texto())
            } else if destructiva {
                (fondo_dest, tinta_dest)
            } else {
                (tema::acento(), tema::sobre_acento())
            };
            let fondo = if self.señalado == Some(es_aceptar) {
                tema::mezclar(base, tema::tinta(), if es_aceptar { 0.10 } else { 0.08 })
            } else {
                base
            };
            let pildora = container(
                text(label.to_string())
                    .size(14.5)
                    .font(Font {
                        weight: Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tinta),
            )
            .center_x(Length::Fixed(ancho_boton - 8.0))
            .center_y(Length::Fixed(ALTO_BOTON - 8.0))
            .style(move |_| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    width: if !foco && tema::alto_contraste() {
                        1.0
                    } else {
                        0.0
                    },
                    color: tema::borde(),
                },
                ..Default::default()
            });

            container(pildora)
                .center_x(Length::Fixed(ancho_boton))
                .center_y(Length::Fixed(ALTO_BOTON))
                .style(move |_| container::Style {
                    border: Border {
                        radius: tema::R_PILL.into(),
                        width: if foco { 2.0 } else { 0.0 },
                        color: tema::texto(),
                    },
                    ..Default::default()
                })
        };

        let mut bloque_cuerpo = column![].align_x(Horizontal::Center);
        for l in &self.lineas {
            bloque_cuerpo = bloque_cuerpo.push(
                container(
                    text(l.clone())
                        .size(13)
                        .color(tema::texto2())
                        .align_x(Horizontal::Center),
                )
                .height(LH_CUERPO)
                .center_x(Length::Fill)
                .center_y(LH_CUERPO),
            );
        }
        if self.lineas.len() < 2 {
            bloque_cuerpo =
                bloque_cuerpo.push(Space::new().height((2 - self.lineas.len()) as f32 * LH_CUERPO));
        }

        let mut contenido = column![
            container(insignia).width(Length::Fill).center_x(Length::Fill),
            Space::new().height(GAP_INSIGNIA_TITULO),
            container(
                text(self.accion.titulo())
                    .size(17)
                    .font(Font {
                        weight: Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto())
                    .align_x(Horizontal::Center)
            )
            .center_x(Length::Fill)
            .center_y(ALTO_TITULO)
            .height(ALTO_TITULO),
            Space::new().height(GAP_TITULO_CUERPO),
            bloque_cuerpo,
        ];

        if let Some(err) = &self.error {
            contenido = contenido.push(Space::new().height(GAP_ERROR)).push(
                container(
                    text(err.clone())
                        .size(11.5)
                        .font(Font {
                            weight: Weight::Medium,
                            ..Font::DEFAULT
                        })
                        .color(tema::rojo_en(tema::card()))
                        .align_x(Horizontal::Center),
                )
                .width(Length::Fill)
                .height(ALTO_ERROR)
                .center_x(Length::Fill)
                .center_y(ALTO_ERROR),
            );
        }

        contenido = contenido.push(Space::new().height(GAP_BOTONES)).push(row![
            boton(t("Cancelar", "Cancel"), false, !self.aceptar),
            Space::new().width(HUECO_BOTONES),
            boton(self.accion.etiqueta(), true, self.aceptar),
        ]);

        let borde_tarjeta = if tema::alto_contraste() {
            1.5
        } else {
            1.0
        };

        container(contenido)
            .padding(iced_core::Padding {
                top: PADDING_TOP,
                right: PADDING_H,
                bottom: PADDING_BOTTOM,
                left: PADDING_H,
            })
            .width(self.ancho)
            .height(alto_total)
            .style(move |_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: R_DIALOGO_CIRCULAR.into(),
                    width: borde_tarjeta,
                    color: tema::borde(),
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
        assert_eq!(c.size(), (Confirmacion::ANCHO, Confirmacion::ALTO));
        assert_eq!(c.pulsar(80.0, 164.0), Some(false));
        assert_eq!(c.pulsar(210.0, 164.0), Some(true));
        assert_eq!(c.pulsar(155.0, 164.0), None);
        assert_eq!(c.pulsar(210.0, 120.0), None);
    }

    #[test]
    fn todas_las_acciones_estandar_caben_en_208() {
        for e in [
            Energia::Suspender,
            Energia::Hibernar,
            Energia::Bloquear,
            Energia::CerrarSesion,
            Energia::Reiniciar,
            Energia::Apagar,
        ] {
            let c = Confirmacion::new(e);
            assert_eq!(
                c.size(),
                (Confirmacion::ANCHO, Confirmacion::ALTO),
                "{e:?} se pasa de dos líneas"
            );
        }
    }

    #[test]
    fn contenido_largo_y_error_crecen_sin_solapar_botones() {
        let mut c = Confirmacion::amplia(
            Energia::Apagar,
            "Hay varias aplicaciones con documentos sin guardar. Si continúas ahora se cerrarán de inmediato y perderás los cambios recientes.",
        );
        let (_, base_h) = c.size();
        assert!(base_h >= Confirmacion::ALTO);
        c.poner_error("No se pudo iniciar el apagado (inhibidor activo).");
        let (_, con_error_h) = c.size();
        assert!(con_error_h > base_h);
        let y_btn = con_error_h - PADDING_BOTTOM - ALTO_BOTON + 10.0;
        assert_eq!(c.pulsar(60.0, y_btn), Some(false));
        assert_eq!(c.pulsar(340.0, y_btn), Some(true));
    }

    #[test]
    fn solo_las_acciones_con_perdida_piden_confirmacion() {
        assert!(!Energia::Suspender.pide_confirmacion());
        assert!(!Energia::Hibernar.pide_confirmacion());
        assert!(!Energia::Bloquear.pide_confirmacion());
        assert!(Energia::CerrarSesion.pide_confirmacion());
        assert!(Energia::Reiniciar.pide_confirmacion());
        assert!(Energia::Apagar.pide_confirmacion());
    }
}
