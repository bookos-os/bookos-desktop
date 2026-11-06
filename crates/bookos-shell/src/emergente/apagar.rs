//! El diálogo del botón de encendido: dormir, bloquear, salir, reiniciar,
//! apagar.
//!
//! Sale con la tecla de encendido del portátil y con `Meta+Esc`. Es la versión
//! de una pantalla del bloque de energía del menú de BookOS: las mismas cinco
//! acciones y los mismos comandos, para que hagan lo mismo se llegue por donde
//! se llegue.
//!
//! **Por qué un diálogo y no apagar directamente.** El botón de encendido de un
//! portátil se roza con la mano al cerrar la tapa y al buscar el enchufe, y
//! apagar sin preguntar es la clase de error que cuesta el trabajo sin guardar.
//! Preguntar cuesta una tecla más —Intro sobre lo que ya está señalado— y
//! convierte un accidente en un Esc.
//!
//! La opción señalada al abrirse es **Dormir**, que es lo que quiere quien
//! toca el botón de encendido nueve de cada diez veces, y además es lo único de
//! la fila que no pierde nada si se pulsa sin querer.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema::{self, Realce};
use crate::view::PanelElement;

use super::{Ancla, Tecla};

const ANCHO: f32 = 468.0;
const MARGEN: f32 = 24.0;
/// Alto del renglón del título.
const TITULO: f32 = 30.0;
/// Aire entre el título y la fila de botones.
const HUECO_TITULO: f32 = 14.0;
/// Medidas de un botón: la celda entera, el disco del icono y el icono.
const BOTON_ANCHO: f32 = 82.0;
const BOTON_ALTO: f32 = 100.0;
const HUECO: f32 = 8.0;
const DISCO: f32 = 52.0;
const ICONO: f32 = 24.0;

/// Una opción del diálogo.
struct Opcion {
    etiqueta: &'static str,
    icono: &'static str,
    /// Si es de las que no tienen vuelta atrás, para pintarla en rojo.
    grave: bool,
    accion: fn() -> Accion,
}

/// Las cinco, en orden de menos a más definitivo: es el orden en que se leen y
/// deja lo irreversible lejos de donde arranca la selección.
// 0.1.0
fn opciones() -> [Opcion; 5] {
    [
        Opcion {
            etiqueta: "Dormir",
            icono: "dormir",
            grave: false,
            // logind decide si hay un inhibidor válido antes de suspender.
            accion: || Accion::Lanzar("systemctl suspend".into()),
        },
        Opcion {
            etiqueta: "Bloquear",
            icono: "bloquear",
            grave: false,
            // La pantalla de bloqueo es la del propio compositor, no la de
            // KDE: el menú todavía llama a `qdbus org.freedesktop.ScreenSaver`
            // porque se escribió antes de que existiera la de aquí, y llamar
            // fuera cuando la tenemos dentro es pedirle a otro escritorio que
            // tape el nuestro.
            accion: || Accion::Bloquear,
        },
        Opcion {
            etiqueta: "Cerrar sesión",
            icono: "salir",
            grave: true,
            accion: || Accion::CerrarSesion,
        },
        Opcion {
            etiqueta: "Reiniciar",
            icono: "reiniciar",
            grave: true,
            accion: || Accion::Lanzar("systemctl reboot".into()),
        },
        Opcion {
            etiqueta: "Apagar",
            icono: "apagar",
            grave: true,
            accion: || Accion::Lanzar("systemctl poweroff".into()),
        },
    ]
}

/// La que sale señalada al abrirse. Ver la cabecera del módulo.
const INICIAL: usize = 0;

pub struct Apagar {
    opciones: [Opcion; 5],
    /// La señalada, con su realce entrando y saliendo. Es la misma para el
    /// ratón y para el teclado: mover el ratón mueve la selección, así que
    /// Intro hace siempre lo que está resaltado.
    señalada: Realce,
    confirmar: Option<crate::confirmacion::Confirmacion>,
}

impl Apagar {
    // 0.1.0
    pub fn confirmar(accion: crate::confirmacion::Energia) -> Self {
        let mut dialogo = Self::new();
        dialogo.confirmar = Some(crate::confirmacion::Confirmacion::new(accion));
        dialogo
    }
    // 0.1.0
    pub fn new() -> Self {
        let mut señalada = Realce::nuevo();
        señalada.señalar(Some(INICIAL));
        señalada.terminar();
        Self {
            opciones: opciones(),
            señalada,
            confirmar: None,
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        if self.confirmar.is_some() {
            return (
                crate::confirmacion::Confirmacion::ANCHO,
                crate::confirmacion::Confirmacion::ALTO,
            );
        }
        (ANCHO, MARGEN * 2.0 + TITULO + HUECO_TITULO + BOTON_ALTO)
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// El velo de detrás. Es un diálogo: lo de debajo se apaga porque hay que
    /// contestar antes de seguir.
    // 0.1.0
    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    /// `x` del borde izquierdo del botón `i`. Los cinco van centrados, con el
    /// sobrante repartido a los lados.
    // 0.1.0
    fn x_boton(&self, i: usize) -> f32 {
        let fila = BOTON_ANCHO * 5.0 + HUECO * 4.0;
        (ANCHO - fila) / 2.0 + i as f32 * (BOTON_ANCHO + HUECO)
    }

    // 0.1.0
    fn y_botones(&self) -> f32 {
        MARGEN + TITULO + HUECO_TITULO
    }

    // 0.1.0
    fn boton_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_botones();
        if y < y0 || y > y0 + BOTON_ALTO {
            return None;
        }
        (0..self.opciones.len()).find(|i| {
            let x0 = self.x_boton(*i);
            x >= x0 && x < x0 + BOTON_ANCHO
        })
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if self.confirmar.is_some() {
            return false;
        }
        // Salir del diálogo **no** apaga la selección: hay que dejar siempre
        // una señalada para que Intro tenga qué hacer. Solo la cambia pasar por
        // encima de otra.
        let Some(nueva) = punto.and_then(|(x, y)| self.boton_en(x, y)) else {
            return false;
        };
        self.señalada.señalar(Some(nueva))
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(c) = &self.confirmar {
            return match c.pulsar(x, y) {
                Some(true) => Some(Accion::EnergiaConfirmada(c.accion)),
                Some(false) => Some(Accion::CerrarEmergente),
                None => None,
            };
        }
        let i = self.boton_en(x, y)?;
        Some((self.opciones[i].accion)())
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        if let Some(c) = &mut self.confirmar {
            return match c.tecla(tecla) {
                Some(true) => Tecla::Hacer(Accion::EnergiaConfirmada(c.accion)),
                Some(false) => Tecla::Cerrar,
                None => Tecla::Consumida,
            };
        }
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda => self.mover(-1),
            T::Derecha => self.mover(1),
            T::Intro => {
                let i = self.señalada.actual().unwrap_or(INICIAL);
                Tecla::Hacer((self.opciones[i].accion)())
            }
            _ => Tecla::Ignorada,
        }
    }

    /// Mueve la selección **sin dar la vuelta**: pasar de «Dormir» a «Apagar»
    /// por el borde izquierdo es justo el salto que no se quiere en una fila
    /// donde el último elemento apaga el equipo.
    // 0.1.0
    fn mover(&mut self, paso: isize) -> Tecla {
        let n = self.opciones.len() as isize;
        let actual = self.señalada.actual().unwrap_or(INICIAL) as isize;
        let siguiente = (actual + paso).clamp(0, n - 1) as usize;
        self.señalada.señalar(Some(siguiente));
        self.confirmar = None;
        Tecla::Consumida
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        if let Some(c) = &self.confirmar {
            return c.view();
        }
        let mut fila = row![];
        for (i, opcion) in self.opciones.iter().enumerate() {
            if i > 0 {
                fila = fila.push(Space::new().width(Length::Fixed(HUECO)));
            }
            fila = fila.push(boton(opcion, self.señalada.intensidad(i)));
        }

        let contenido = column![
            container(
                text("¿Qué quieres hacer?")
                    .size(tema::T_TITULO)
                    .color(tema::texto())
            )
            .width(Length::Fill)
            .height(Length::Fixed(TITULO))
            .center_x(Length::Fill)
            .center_y(Length::Fixed(TITULO)),
            Space::new().height(Length::Fixed(HUECO_TITULO)),
            container(fila).width(Length::Fill).center_x(Length::Fill),
        ];

        super::control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Un botón: disco con el icono arriba y su etiqueta debajo.
// 0.1.0
fn boton<'a>(opcion: &'a Opcion, señalado: f32) -> PanelElement<'a> {
    // La señalada se rellena del acento —o del rojo de peligro, si es de las
    // que no tienen vuelta atrás— y su icono pasa a la tinta que se lea encima.
    // Las demás se quedan en el relleno tenue de siempre.
    let lleno = if opcion.grave {
        tema::rojo()
    } else {
        tema::acento()
    };
    let fondo = tema::mezclar(tema::alfa(tema::tinta(), 0.08), lleno, señalado);
    let tinta = tema::mezclar(tema::texto(), tema::tinta_sobre(lleno), señalado);

    let dibujo: PanelElement<'a> = match crate::icono::propio(opcion.icono) {
        Some(ic) => crate::icono::ver_teñido_propio(&ic, ICONO, tinta),
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let disco = container(dibujo)
        .width(Length::Fixed(DISCO))
        .height(Length::Fixed(DISCO))
        .center_x(Length::Fixed(DISCO))
        .center_y(Length::Fixed(DISCO))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    container(
        column![
            disco,
            Space::new().height(Length::Fixed(8.0)),
            text(opcion.etiqueta)
                .size(tema::T_PEQUENO)
                // La etiqueta sube de gris a tinta con el realce: es lo que
                // dice cuál se va a ejecutar al pulsar Intro.
                .color(tema::mezclar(tema::TEXTO2, tema::texto(), señalado)),
        ]
        .align_x(Horizontal::Center),
    )
    .width(Length::Fixed(BOTON_ANCHO))
    .height(Length::Fixed(BOTON_ALTO))
    .align_x(Horizontal::Center)
    .align_y(Vertical::Top)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los cinco botones caben en la tarjeta y cada uno responde en su sitio.
    #[test]
    fn los_cinco_botones_caen_donde_se_dibujan() {
        let a = Apagar::new();
        let y = a.y_botones() + 10.0;
        for i in 0..5 {
            let centro = a.x_boton(i) + BOTON_ANCHO / 2.0;
            assert_eq!(a.boton_en(centro, y), Some(i));
            assert!(a.x_boton(i) >= 0.0 && a.x_boton(i) + BOTON_ANCHO <= ANCHO);
        }
        assert_eq!(a.boton_en(2.0, y), None, "el margen no es de nadie");
        assert_eq!(a.boton_en(ANCHO / 2.0, 4.0), None, "el título tampoco");
        assert!(a.y_botones() + BOTON_ALTO <= a.size().1);
    }

    /// Arranca en «Dormir» y las flechas no dan la vuelta: llegar a «Apagar»
    /// desde el principio son cuatro pulsaciones, no una hacia atrás.
    #[test]
    fn la_seleccion_no_da_la_vuelta() {
        let mut a = Apagar::new();
        assert_eq!(a.señalada.actual(), Some(0));
        a.tecla(crate::TeclaPulsada::Izquierda);
        assert_eq!(
            a.señalada.actual(),
            Some(0),
            "a la izquierda de la primera, ella"
        );
        for _ in 0..9 {
            a.tecla(crate::TeclaPulsada::Derecha);
        }
        assert_eq!(
            a.señalada.actual(),
            Some(4),
            "y a la derecha de la última, ella"
        );
    }

    /// Intro ejecuta lo que está señalado, y con el ratón encima ejecuta eso.
    #[test]
    fn intro_hace_lo_señalado() {
        let mut a = Apagar::new();
        assert!(
            matches!(a.tecla(crate::TeclaPulsada::Intro), Tecla::Hacer(Accion::Lanzar(cmd)) if cmd.contains("suspend"))
        );
        // Con el puntero sobre «Bloquear», Intro bloquea.
        let y = a.y_botones() + 10.0;
        a.puntero(Some((a.x_boton(1) + 4.0, y)));
        assert!(matches!(
            a.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::Bloquear)
        ));
        // Y pulsar «Apagar» pide apagar.
        let accion = a.pulsar(a.x_boton(4) + 4.0, y);
        assert!(matches!(accion, Some(Accion::Lanzar(cmd)) if cmd.contains("poweroff")));
    }

    /// Salir del diálogo con el ratón no puede dejarlo sin selección: Intro
    /// tiene que seguir teniendo qué hacer.
    #[test]
    fn el_raton_fuera_no_apaga_la_seleccion() {
        let mut a = Apagar::new();
        a.puntero(Some((a.x_boton(3) + 4.0, a.y_botones() + 10.0)));
        assert_eq!(a.señalada.actual(), Some(3));
        a.puntero(None);
        assert_eq!(a.señalada.actual(), Some(3));
    }
}
