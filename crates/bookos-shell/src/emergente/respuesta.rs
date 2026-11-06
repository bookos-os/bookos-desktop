//! La caja para responder a una notificación sin abrir su aplicación.
//!
//! La abre el botón «Responder» de un aviso, que es el que las aplicaciones
//! piden con la acción `inline-reply` —la convención de KDE Connect y de
//! Telegram—. El aviso no puede llevar el campo dentro: es una superficie del
//! shell sin foco de teclado, y las emergentes sí lo reciben. Intro manda el
//! texto de vuelta como [`Accion::NotificacionRespuesta`] y el compositor lo
//! entrega por la señal `NotificationReplied`.

use iced_core::{Border, Length};
use iced_widget::{Space, column, container, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, Tecla, t};

const ANCHO: f32 = 440.0;
const CAMPO: f32 = 56.0;
const PIE: f32 = 28.0;
/// Alto de la línea con quién escribe y a qué responde.
const CABECERA: f32 = 40.0;
const MARGEN: f32 = 20.0;

pub struct Respuesta {
    id: u32,
    /// Quién avisó y de qué: «Lumina · Ana».
    contexto: String,
    texto: String,
}

impl Respuesta {
    // 0.1.0
    pub fn new(id: u32, app: &str, resumen: &str) -> Self {
        Self {
            id,
            contexto: format!("{app} · {resumen}"),
            texto: String::new(),
        }
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (ANCHO, CABECERA + CAMPO + PIE)
    }

    /// El mismo velo que el buscador: pide atención sin apagar el escritorio.
    // 0.1.0
    pub fn velo(&self) -> iced_core::Color {
        tema::alfa(iced_core::Color::BLACK, 0.45)
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        false
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            // Esc borra lo escrito antes de cerrar, como el buscador: un
            // aviso de mensajería se contesta a medias muchas veces.
            T::Escape if !self.texto.is_empty() => {
                self.texto.clear();
                Tecla::Consumida
            }
            T::Escape => Tecla::Cerrar,
            T::Caracter(c) => {
                self.texto.push(c);
                Tecla::Consumida
            }
            T::Retroceso => {
                self.texto.pop();
                Tecla::Consumida
            }
            // Un mensaje en blanco no se manda: Intro sin nada escrito es un
            // dedo que resbala, no una respuesta.
            T::Intro if self.texto.trim().is_empty() => Tecla::Consumida,
            T::Intro => Tecla::Hacer(Accion::NotificacionRespuesta {
                id: self.id,
                texto: self.texto.trim().to_string(),
            }),
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let ancho_texto = ANCHO - MARGEN * 2.0;
        let escrito: PanelElement<'_> = if self.texto.is_empty() {
            text(t("Escribe tu respuesta", "Type your reply"))
                .size(16.0)
                .color(tema::TEXTO2)
                .into()
        } else {
            // Sin parpadeo, por lo mismo que el buscador: dos repintados por
            // segundo de una tarjeta quieta es un despertar que no se paga.
            text(super::recortar_texto(
                &format!("{}▏", self.texto),
                ancho_texto,
                16.0,
            ))
            .size(16.0)
            .color(tema::texto())
            .into()
        };
        let contenido = column![
            container(
                text(super::recortar_texto(&self.contexto, ancho_texto, 13.0))
                    .size(13.0)
                    .color(tema::TEXTO2)
            )
            .padding([0, MARGEN as u16])
            .height(Length::Fixed(CABECERA))
            .align_y(iced_core::alignment::Vertical::Center),
            container(escrito)
                .padding([0, MARGEN as u16])
                .width(Length::Fixed(ANCHO))
                .height(Length::Fixed(CAMPO))
                .align_y(iced_core::alignment::Vertical::Center),
            container(
                text(t("Intro envía · Esc cancela", "Enter sends · Esc cancels"))
                    .size(11.0)
                    .color(tema::TEXTO2)
            )
            .padding([0, MARGEN as u16])
            .height(Length::Fixed(PIE))
            .align_y(iced_core::alignment::Vertical::Center),
            Space::new(),
        ];
        container(contenido)
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(CABECERA + CAMPO + PIE))
            .style(|_theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
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
    use crate::TeclaPulsada as T;

    fn escribir(r: &mut Respuesta, s: &str) {
        for c in s.chars() {
            r.tecla(T::Caracter(c));
        }
    }

    #[test]
    fn intro_manda_lo_escrito_a_la_notificacion_correcta() {
        let mut r = Respuesta::new(7, "Lumina", "Ana");
        escribir(&mut r, " hola ");
        match r.tecla(T::Intro) {
            Tecla::Hacer(Accion::NotificacionRespuesta { id, texto }) => {
                assert_eq!((id, texto.as_str()), (7, "hola"));
            }
            _ => panic!("Intro tenía que mandar la respuesta"),
        }
    }

    #[test]
    fn un_mensaje_en_blanco_no_se_manda() {
        let mut r = Respuesta::new(7, "Lumina", "Ana");
        escribir(&mut r, "  ");
        assert!(matches!(r.tecla(T::Intro), Tecla::Consumida));
    }

    #[test]
    fn escape_borra_y_a_la_segunda_cierra() {
        let mut r = Respuesta::new(7, "Lumina", "Ana");
        escribir(&mut r, "hol");
        assert!(matches!(r.tecla(T::Escape), Tecla::Consumida));
        assert!(matches!(r.tecla(T::Escape), Tecla::Cerrar));
    }
}
