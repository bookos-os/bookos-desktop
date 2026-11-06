//! Diálogo de color de carpeta: el mismo de bookos-explorer (`pickColor`).
//!
//! Sustituye a los círculos de 14 px que colgaban del menú contextual: aplicaban
//! el color al primer clic, sin ver el resultado ni poder volver atrás, y no
//! daban acceso al color libre que sí ofrece el explorador.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length, Padding, Radians, Shadow, Vector, gradient::Linear};
use iced_widget::{Space, column, container, row, stack, text};

use super::{Ancla, Tecla};
use crate::escritorio::{MUESTRAS, icono_carpeta};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

/// Las medidas salen de `.dlg`, `.sw`, `.fico-big` y `.custom-color` del
/// explorador, para que los dos diálogos se lean como el mismo.
const SOMBRA: f32 = 22.0;
const ANCHO: f32 = 310.0;
const LATERAL: f32 = 24.0;
const ARRIBA: f32 = 28.0;
const ABAJO: f32 = 20.0;
const ICONO_W: f32 = 104.0;
const ICONO_H: f32 = 86.0;
const TITULO: f32 = 24.0;
const Y_MUESTRAS: f32 = ARRIBA + ICONO_H + 12.0 + TITULO + 18.0;
const MUESTRA: f32 = 30.0;
const HUECO_MUESTRA: f32 = 8.0;
const AIRE_BOTONES: f32 = 22.0;
const BOTON_ALTO: f32 = 48.0;
const HUECO_BOTON: f32 = 10.0;
const INTERIOR: f32 = ANCHO - LATERAL * 2.0;
const PANEL_MARGEN: f32 = 13.0;
const PANEL_RELLENO: f32 = 10.0;
const PANEL_ANCHO: f32 = INTERIOR - PANEL_RELLENO * 2.0;
const SUPERFICIE_ALTO: f32 = 100.0;
const TONO_ALTO: f32 = 14.0;
const HEX_ALTO: f32 = 16.0;
const HUECO_PANEL: f32 = 12.0;
const PANEL_ALTO: f32 =
    PANEL_RELLENO * 2.0 + SUPERFICIE_ALTO + TONO_ALTO + HEX_ALTO + HUECO_PANEL * 2.0;
const PUNTO: f32 = 18.0;
const POMO: f32 = 20.0;
/// El azul con el que el explorador abre el campo si no hay color puesto.
const HEX_INICIAL: &str = "#007AFF";
/// La escala del panel de BookOS. En otra, iced reescala una imagen que es puro
/// degradado y no se nota.
const ESCALA_SOMBRA: f32 = 2.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Boton {
    Cancelar,
    Aceptar,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Agarre {
    Superficie,
    Tono,
}

struct Libre {
    tono: f32,
    sat: f32,
    val: f32,
    hex: String,
    /// Recién abierto el campo está seleccionado, como con `select()` en el
    /// explorador: la primera tecla sustituye en vez de añadir.
    hex_seleccionado: bool,
}

pub struct ColorCarpeta {
    nombre: String,
    titulo: String,
    color: Option<String>,
    icono: Icono,
    marca: Icono,
    mas: Icono,
    sombra: [Icono; 4],
    libre: Option<Libre>,
    agarre: Option<Agarre>,
    señalado: Option<Boton>,
}

fn hsv_a_hex(tono: f32, sat: f32, val: f32) -> String {
    let c = val * sat;
    let h = tono.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let canal = |v: f32| ((v + val - c) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", canal(r), canal(g), canal(b))
}

fn rgb(hex: &str) -> Option<u32> {
    let cifras = hex.strip_prefix('#')?;
    (cifras.len() == 6)
        .then(|| u32::from_str_radix(cifras, 16).ok())
        .flatten()
}

fn hex_a_hsv(hex: &str) -> Option<(f32, f32, f32)> {
    let n = rgb(hex)?;
    let canal = |d: u32| ((n >> d) & 0xff) as f32 / 255.0;
    let (r, g, b) = (canal(16), canal(8), canal(0));
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let tono = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Some((tono, if max == 0.0 { 0.0 } else { d / max }, max))
}

/// La sombra de la tarjeta, rasterizada una vez con la misma fórmula de
/// `iced_tiny_skia::engine::draw_quad`: arriba, izquierda, derecha y abajo.
///
/// Como `Shadow` de un `container`, iced recalcula la distancia de cada píxel en
/// cada repintado aunque la tarjeta tape casi todos, y una sola imagen del búfer
/// entero sigue mezclándolos. Solo se guarda lo que asoma: el margen y, arriba y
/// abajo, el radio, porque la sombra se ve por las esquinas redondeadas.
///
/// Repintado con el selector abierto (708×1038, a escala 2, con una pausa entre
/// repintados como en un arrastre real): 45 ms con `Shadow`, 19 con una imagen
/// entera, 15,5 con estas cuatro y 11 sin ninguna sombra.
// añadido en la 0.2.0
fn sombra_rasterizada(alto_tarjeta: f32) -> [Icono; 4] {
    let sombra = tema::sombra_popover();
    let px = |logico: f32| logico * ESCALA_SOMBRA;
    let (medio_x, medio_y) = (px(ANCHO) / 2.0, px(alto_tarjeta) / 2.0);
    let (centro_x, centro_y) = (px(SOMBRA) + medio_x, px(SOMBRA + sombra.offset.y) + medio_y);
    let (radio, difuminado) = (px(tema::R_DIALOGO), px(sombra.blur_radius));
    let tira = |x0: f32, y0: f32, ancho: f32, alto: f32| {
        let (x0, y0, ancho, alto) = (
            px(x0) as u32,
            px(y0) as u32,
            px(ancho) as u32,
            px(alto) as u32,
        );
        let rgba: Vec<u8> = (y0..y0 + alto)
            .flat_map(|y| (x0..x0 + ancho).map(move |x| (x as f32, y as f32)))
            .flat_map(|(x, y)| {
                let dx = ((x - centro_x).abs() - medio_x + radio).max(0.0);
                let dy = ((y - centro_y).abs() - medio_y + radio).max(0.0);
                let distancia = (dx.hypot(dy) - radio).max(0.0);
                let t = ((distancia + difuminado) / (difuminado * 2.0)).clamp(0.0, 1.0);
                let alfa = sombra.color.a * (1.0 - t * t * (3.0 - 2.0 * t));
                [0, 0, 0, (alfa * 255.0).round() as u8]
            })
            .collect();
        Icono::Raster(iced_core::image::Handle::from_rgba(ancho, alto, rgba))
    };
    let ancho = ANCHO + SOMBRA * 2.0;
    let franja = SOMBRA + tema::R_DIALOGO;
    let lado = alto_tarjeta + SOMBRA * 2.0 - franja * 2.0;
    [
        tira(0.0, 0.0, ancho, franja),
        tira(0.0, franja, SOMBRA, lado),
        tira(SOMBRA + ANCHO, franja, SOMBRA, lado),
        tira(0.0, franja + lado, ancho, franja),
    ]
}

fn color_de(hex: &str) -> Color {
    tema::hex(rgb(hex).expect("los colores del diálogo pasan por hex_valido"))
}

impl ColorCarpeta {
    // añadido en la 0.2.0
    pub fn new(carpetas: &[(&str, Option<&str>)]) -> Self {
        let (nombre, color) = carpetas
            .first()
            .copied()
            .expect("el menú solo ofrece el color con carpetas seleccionadas");
        let titulo = match carpetas.len() {
            1 if nombre.chars().count() > 30 => {
                format!("{}…", nombre.chars().take(29).collect::<String>())
            }
            1 => nombre.to_string(),
            n => format!("{n} carpetas"),
        };
        let tinta = tema::texto();
        Self {
            nombre: nombre.to_string(),
            titulo,
            color: color.map(str::to_string),
            icono: icono_carpeta(nombre, color),
            marca: icono::desde_svg(
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/></svg>"##,
            ),
            mas: icono::desde_svg(&format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" fill="none" stroke="#{:02X}{:02X}{:02X}" stroke-width="2.4" stroke-linecap="round"/></svg>"##,
                (tinta.r * 255.0) as u8,
                (tinta.g * 255.0) as u8,
                (tinta.b * 255.0) as u8,
            )),
            sombra: sombra_rasterizada(Self::alto_para(false)),
            libre: None,
            agarre: None,
            señalado: None,
        }
    }

    fn alto_para(con_selector: bool) -> f32 {
        let panel = if con_selector {
            PANEL_MARGEN + PANEL_ALTO
        } else {
            0.0
        };
        Y_MUESTRAS + MUESTRA + panel + AIRE_BOTONES + BOTON_ALTO + ABAJO
    }

    fn alto(&self) -> f32 {
        Self::alto_para(self.libre.is_some())
    }

    // añadido en la 0.2.0
    pub fn size(&self) -> (f32, f32) {
        (ANCHO + SOMBRA * 2.0, self.alto() + SOMBRA * 2.0)
    }

    // añadido en la 0.2.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    // añadido en la 0.2.0
    pub fn animando(&self) -> bool {
        false
    }

    /// Con el selector abierto el «+» desaparece y quedan seis, como en el
    /// explorador.
    fn casillas(&self) -> usize {
        MUESTRAS.len() + usize::from(self.libre.is_none())
    }

    fn x_muestra(&self, i: usize) -> f32 {
        let n = self.casillas() as f32;
        let total = n * MUESTRA + (n - 1.0) * HUECO_MUESTRA;
        (ANCHO - total) / 2.0 + i as f32 * (MUESTRA + HUECO_MUESTRA)
    }

    fn muestra_en(&self, x: f32, y: f32) -> Option<usize> {
        if !(Y_MUESTRAS..Y_MUESTRAS + MUESTRA).contains(&y) {
            return None;
        }
        (0..self.casillas()).find(|i| {
            let x0 = self.x_muestra(*i);
            (x0..x0 + MUESTRA).contains(&x)
        })
    }

    fn y_superficie() -> f32 {
        Y_MUESTRAS + MUESTRA + PANEL_MARGEN + PANEL_RELLENO
    }

    fn y_tono() -> f32 {
        Self::y_superficie() + SUPERFICIE_ALTO + HUECO_PANEL
    }

    fn x_panel() -> f32 {
        LATERAL + PANEL_RELLENO
    }

    fn agarre_en(&self, x: f32, y: f32) -> Option<Agarre> {
        self.libre.as_ref()?;
        if !(Self::x_panel()..Self::x_panel() + PANEL_ANCHO).contains(&x) {
            return None;
        }
        if (Self::y_superficie()..Self::y_superficie() + SUPERFICIE_ALTO).contains(&y) {
            Some(Agarre::Superficie)
        } else if (Self::y_tono()..Self::y_tono() + TONO_ALTO).contains(&y) {
            Some(Agarre::Tono)
        } else {
            None
        }
    }

    fn boton_en(&self, x: f32, y: f32) -> Option<Boton> {
        let y0 = self.alto() - ABAJO - BOTON_ALTO;
        if !(y0..y0 + BOTON_ALTO).contains(&y) {
            return None;
        }
        let ancho = (INTERIOR - HUECO_BOTON) / 2.0;
        if (LATERAL..LATERAL + ancho).contains(&x) {
            Some(Boton::Cancelar)
        } else if (LATERAL + ancho + HUECO_BOTON..ANCHO - LATERAL).contains(&x) {
            Some(Boton::Aceptar)
        } else {
            None
        }
    }

    fn poner(&mut self, color: Option<String>) {
        self.icono = icono_carpeta(&self.nombre, color.as_deref());
        if let (Some(libre), Some(hex)) = (self.libre.as_mut(), color.as_deref()) {
            libre.hex = hex.to_string();
            libre.hex_seleccionado = true;
            if let Some((tono, sat, val)) = hex_a_hsv(hex) {
                // El negro y el gris no tienen tono: conservar el de la tira
                // evita que salte al rojo al arrastrar por la esquina oscura.
                if sat > 0.0 && val > 0.0 {
                    libre.tono = tono;
                }
                libre.sat = sat;
                libre.val = val;
            }
        }
        self.color = color;
    }

    fn arrastrar(&mut self, agarre: Agarre, x: f32, y: f32) {
        let Some(libre) = self.libre.as_mut() else {
            return;
        };
        let rel_x = ((x - Self::x_panel()) / PANEL_ANCHO).clamp(0.0, 1.0);
        match agarre {
            Agarre::Superficie => {
                libre.sat = rel_x;
                libre.val = 1.0 - ((y - Self::y_superficie()) / SUPERFICIE_ALTO).clamp(0.0, 1.0);
            }
            Agarre::Tono => libre.tono = rel_x * 360.0,
        }
        let hex = hsv_a_hex(libre.tono, libre.sat, libre.val);
        libre.hex.clone_from(&hex);
        libre.hex_seleccionado = true;
        self.icono = icono_carpeta(&self.nombre, Some(&hex));
        self.color = Some(hex);
    }

    // añadido en la 0.2.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let punto = punto.map(|(x, y)| (x - SOMBRA, y - SOMBRA));
        if let (Some(agarre), Some((x, y))) = (self.agarre, punto) {
            self.arrastrar(agarre, x, y);
            return true;
        }
        let señalado = punto.and_then(|(x, y)| self.boton_en(x, y));
        let cambia = señalado != self.señalado;
        self.señalado = señalado;
        cambia
    }

    // añadido en la 0.2.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (x, y) = (x - SOMBRA, y - SOMBRA);
        match self.boton_en(x, y) {
            Some(Boton::Cancelar) => return Some(Accion::CerrarEmergente),
            Some(Boton::Aceptar) => return Some(Accion::EscritorioColor(self.color.clone())),
            None => {}
        }
        if let Some(i) = self.muestra_en(x, y) {
            match MUESTRAS.get(i) {
                // Pulsar la activa la apaga: se vuelve al automático sin gastar
                // otro círculo en ello, igual que en el explorador.
                Some(hex) if self.color.as_deref() == Some(hex) => self.poner(None),
                Some(hex) => self.poner(Some((*hex).to_string())),
                None => self.abrir_libre(),
            }
        } else if let Some(agarre) = self.agarre_en(x, y) {
            self.agarre = Some(agarre);
            self.arrastrar(agarre, x, y);
        }
        None
    }

    fn abrir_libre(&mut self) {
        let hex = self.color.as_deref().unwrap_or(HEX_INICIAL);
        let (tono, sat, val) =
            hex_a_hsv(hex).expect("los colores del diálogo pasan por hex_valido");
        self.libre = Some(Libre {
            tono,
            sat,
            val,
            hex: hex.to_string(),
            hex_seleccionado: true,
        });
        self.sombra = sombra_rasterizada(self.alto());
    }

    // añadido en la 0.2.0
    pub fn soltar(&mut self) -> bool {
        self.agarre.take().is_some()
    }

    // añadido en la 0.2.0
    pub fn agarrado(&self) -> bool {
        self.agarre.is_some()
    }

    // añadido en la 0.2.0
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        self.agarre_en(x - SOMBRA, y - SOMBRA).is_some()
    }

    fn mover_muestra(&mut self, paso: usize) {
        let n = MUESTRAS.len();
        let actual = self
            .color
            .as_deref()
            .and_then(|c| MUESTRAS.iter().position(|m| *m == c));
        let i = actual.map_or(if paso == 1 { 0 } else { n - 1 }, |i| (i + paso) % n);
        self.poner(Some(MUESTRAS[i].to_string()));
    }

    fn teclear_hex(&mut self, tecla: TeclaPulsada) -> Tecla {
        let Some(libre) = self.libre.as_mut() else {
            return Tecla::Ignorada;
        };
        match tecla {
            TeclaPulsada::Caracter(c) if c == '#' || c.is_ascii_hexdigit() => {
                if libre.hex_seleccionado {
                    libre.hex.clear();
                    libre.hex_seleccionado = false;
                }
                if libre.hex.is_empty() && c != '#' {
                    libre.hex.push('#');
                }
                if libre.hex.len() < 7 && (c != '#' || libre.hex.is_empty()) {
                    libre.hex.push(c.to_ascii_uppercase());
                }
            }
            TeclaPulsada::Retroceso => {
                libre.hex_seleccionado = false;
                libre.hex.pop();
            }
            _ => return Tecla::Ignorada,
        }
        // Un hex a medias no es un color: se queda en el campo hasta completarse.
        if rgb(&libre.hex).is_some() {
            let hex = libre.hex.clone();
            self.poner(Some(hex));
            if let Some(libre) = self.libre.as_mut() {
                libre.hex_seleccionado = false;
            }
        }
        Tecla::Consumida
    }

    // añadido en la 0.2.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        match tecla {
            TeclaPulsada::Escape => Tecla::Cerrar,
            TeclaPulsada::Intro => Tecla::Hacer(Accion::EscritorioColor(self.color.clone())),
            TeclaPulsada::Izquierda => {
                self.mover_muestra(MUESTRAS.len() - 1);
                Tecla::Consumida
            }
            TeclaPulsada::Derecha => {
                self.mover_muestra(1);
                Tecla::Consumida
            }
            otra => self.teclear_hex(otra),
        }
    }

    fn fila_muestras(&self) -> PanelElement<'_> {
        let mut fila = row![].spacing(HUECO_MUESTRA);
        for hex in MUESTRAS {
            let color = color_de(hex);
            let dentro: PanelElement<'_> = if self.color.as_deref() == Some(hex) {
                icono::ver(&self.marca, 16.0, 16.0)
            } else {
                Space::new().into()
            };
            fila = fila.push(
                container(dentro)
                    .width(MUESTRA)
                    .height(MUESTRA)
                    .center(MUESTRA)
                    .style(move |_| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            radius: tema::R_PILL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        if self.libre.is_none() {
            fila = fila.push(
                container(icono::ver(&self.mas, 16.0, 16.0))
                    .width(MUESTRA)
                    .height(MUESTRA)
                    .center(MUESTRA)
                    .style(|_| container::Style {
                        background: Some(tema::superficie_en_tarjeta().into()),
                        border: Border {
                            radius: tema::R_PILL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        fila.into()
    }

    fn panel_libre<'a>(&'a self, libre: &'a Libre) -> PanelElement<'a> {
        let puro = color_de(&hsv_a_hex(libre.tono, 1.0, 1.0));
        let radio: iced_core::border::Radius = 9.0.into();
        let color_superficie = container(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(
                        Linear::new(Radians(std::f32::consts::PI))
                            .add_stop(0.0, Color::TRANSPARENT)
                            .add_stop(1.0, Color::BLACK)
                            .into(),
                    ),
                    border: Border {
                        radius: radio,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(PANEL_ANCHO)
        .height(SUPERFICIE_ALTO)
        .style(move |_| container::Style {
            background: Some(
                Linear::new(Radians(std::f32::consts::FRAC_PI_2))
                    .add_stop(0.0, Color::WHITE)
                    .add_stop(1.0, puro)
                    .into(),
            ),
            border: Border {
                radius: radio,
                ..Default::default()
            },
            ..Default::default()
        });
        let punto = container(Space::new())
            .width(PUNTO)
            .height(PUNTO)
            .style(|_| container::Style {
                border: Border {
                    radius: tema::R_PILL.into(),
                    width: 3.0,
                    color: Color::WHITE,
                },
                // Sin ella el aro blanco desaparece en la esquina blanca.
                shadow: Shadow {
                    color: Color::BLACK.scale_alpha(0.7),
                    offset: Vector::new(0.0, 1.0),
                    blur_radius: 5.0,
                },
                ..Default::default()
            });
        // La primera capa de un `stack` fija su tamaño y aplasta lo que se salga:
        // el margen le reserva al marcador medio lado por fuera en cada borde.
        let superficie = stack![
            container(color_superficie).padding(PUNTO / 2.0),
            container(punto).padding(Padding {
                left: libre.sat * PANEL_ANCHO,
                top: (1.0 - libre.val) * SUPERFICIE_ALTO,
                ..Default::default()
            })
        ];
        let tira = container(Space::new())
            .width(PANEL_ANCHO)
            .height(TONO_ALTO)
            .style(|_| container::Style {
                background: Some(
                    [
                        0xFF0000, 0xFFFF00, 0x00FF00, 0x00FFFF, 0x0000FF, 0xFF00FF, 0xFF0000,
                    ]
                    .into_iter()
                    .enumerate()
                    .fold(
                        Linear::new(Radians(std::f32::consts::FRAC_PI_2)),
                        |g, (i, c)| g.add_stop(i as f32 / 6.0, tema::hex(c)),
                    )
                    .into(),
                ),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let disco = |lado: f32, color: Color| {
            container(Space::new())
                .width(lado)
                .height(lado)
                .style(move |_| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: (lado / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        let pomo = stack![
            disco(POMO, tema::card()),
            container(disco(POMO - 6.0, Color::WHITE)).padding(3)
        ];
        let tono = stack![
            container(tira).padding(Padding {
                left: POMO / 2.0,
                right: POMO / 2.0,
                top: (POMO - TONO_ALTO) / 2.0,
                bottom: (POMO - TONO_ALTO) / 2.0,
            }),
            container(pomo).padding(Padding {
                left: libre.tono / 360.0 * PANEL_ANCHO,
                ..Default::default()
            })
        ];
        let cursor = if libre.hex_seleccionado { "" } else { "|" };
        let hex = container(
            row![
                text("HEX").size(10).color(tema::TEXTO2),
                Space::new().width(Length::Fill),
                text(format!("{}{cursor}", libre.hex))
                    .size(tema::T_PEQUENO)
                    .color(tema::texto()),
            ]
            .align_y(Vertical::Center),
        )
        .width(PANEL_ANCHO)
        .height(HEX_ALTO);
        // Los márgenes de los marcadores ocupan los huecos y el relleno: lo que
        // queda hace que superficie y tira caigan en las `y` de `agarre_en`.
        container(
            column![
                superficie,
                tono,
                Space::new().height(HUECO_PANEL - (POMO - TONO_ALTO) / 2.0),
                hex,
            ]
            .width(Length::Fill)
            .align_x(Horizontal::Center),
        )
        .padding(Padding {
            top: PANEL_RELLENO - PUNTO / 2.0,
            bottom: PANEL_RELLENO,
            ..Default::default()
        })
        .width(INTERIOR)
        .height(PANEL_ALTO)
        .style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_BOTON.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn boton(&self, etiqueta: &'static str, cual: Boton) -> PanelElement<'_> {
        let principal = cual == Boton::Aceptar;
        let base = if principal {
            tema::acento()
        } else {
            tema::superficie_en_tarjeta()
        };
        let fondo = if self.señalado == Some(cual) {
            tema::mezclar(base, tema::tinta(), 0.12)
        } else {
            base
        };
        let tinta = if principal {
            tema::tinta_sobre(tema::acento())
        } else {
            tema::texto()
        };
        container(text(etiqueta).size(15).color(tinta))
            // `center` fija también el tamaño: tiene que ir antes de `width` y `height`.
            .center(Length::Fill)
            .width((INTERIOR - HUECO_BOTON) / 2.0)
            .height(BOTON_ALTO)
            .style(move |_| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // añadido en la 0.2.0
    // modificado en la 0.2.0 -- arrastrar por el selector repinta en 15,5 ms y no en 45: la sombra ya no se recalcula
    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![
            Space::new().height(ARRIBA),
            icono::ver(&self.icono, ICONO_W, ICONO_H),
            Space::new().height(12),
            container(text(self.titulo.as_str()).size(17).color(tema::texto()))
                .height(TITULO)
                .center_y(TITULO),
            Space::new().height(18),
            self.fila_muestras(),
        ]
        .width(Length::Fill)
        .align_x(Horizontal::Center);
        if let Some(libre) = &self.libre {
            col = col
                .push(Space::new().height(PANEL_MARGEN))
                .push(self.panel_libre(libre));
        }
        col = col.push(Space::new().height(AIRE_BOTONES)).push(
            row![
                self.boton("Cancelar", Boton::Cancelar),
                self.boton("Aceptar", Boton::Aceptar)
            ]
            .spacing(HUECO_BOTON),
        );
        let tarjeta = container(col)
            .width(ANCHO)
            .height(self.alto())
            .style(|_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            });
        let (ancho, alto) = self.size();
        let franja = SOMBRA + tema::R_DIALOGO;
        let lado = alto - franja * 2.0;
        let [arriba, izquierda, derecha, abajo] = &self.sombra;
        stack![
            column![
                icono::ver(arriba, ancho, franja),
                row![
                    icono::ver(izquierda, SOMBRA, lado),
                    Space::new().width(ANCHO),
                    icono::ver(derecha, SOMBRA, lado),
                ],
                icono::ver(abajo, ancho, franja),
            ],
            container(tarjeta).padding(SOMBRA)
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialogo(color: Option<&str>) -> ColorCarpeta {
        ColorCarpeta::new(&[("Proyectos", color)])
    }

    fn centro_muestra(d: &ColorCarpeta, i: usize) -> (f32, f32) {
        (
            SOMBRA + d.x_muestra(i) + MUESTRA / 2.0,
            SOMBRA + Y_MUESTRAS + MUESTRA / 2.0,
        )
    }

    #[test]
    fn elegir_no_aplica_hasta_aceptar_y_cancelar_no_toca_nada() {
        let mut d = dialogo(None);
        let (x, y) = centro_muestra(&d, 1);
        assert_eq!(d.pulsar(x, y), None, "probar un color no lo aplica");
        let y_boton = SOMBRA + d.alto() - ABAJO - 1.0;
        assert_eq!(
            d.pulsar(SOMBRA + ANCHO - LATERAL - 1.0, y_boton),
            Some(Accion::EscritorioColor(Some("#FF3B30".into())))
        );
        assert_eq!(
            d.pulsar(SOMBRA + LATERAL + 1.0, y_boton),
            Some(Accion::CerrarEmergente)
        );
    }

    #[test]
    fn pulsar_la_activa_vuelve_al_automatico() {
        let mut d = dialogo(Some("#FF3B30"));
        let (x, y) = centro_muestra(&d, 1);
        d.pulsar(x, y);
        assert_eq!(d.color, None);
    }

    #[test]
    fn el_mas_abre_el_selector_y_arrastrar_da_un_color_libre() {
        let mut d = dialogo(None);
        let alto = d.alto();
        let (x, y) = centro_muestra(&d, MUESTRAS.len());
        d.pulsar(x, y);
        assert!(d.libre.is_some());
        assert!(d.alto() > alto, "el diálogo crece para el selector");
        assert_eq!(d.casillas(), MUESTRAS.len(), "el «+» desaparece");

        // Esquina superior izquierda de la superficie: blanco.
        let (px, py) = (
            SOMBRA + ColorCarpeta::x_panel() + 1.0,
            SOMBRA + ColorCarpeta::y_superficie() + 1.0,
        );
        d.pulsar(px, py);
        assert!(d.agarrado());
        // Fuera por abajo sigue arrastrando, recortado: negro.
        d.puntero(Some((px, 5000.0)));
        assert_eq!(d.color.as_deref(), Some("#000000"));
        assert!(d.soltar());
    }

    #[test]
    fn el_hex_se_escribe_y_solo_completo_es_color() {
        let mut d = dialogo(None);
        d.abrir_libre();
        for c in "34c75".chars() {
            d.tecla(TeclaPulsada::Caracter(c));
        }
        assert_eq!(d.color, None, "a medias no aplica");
        d.tecla(TeclaPulsada::Caracter('9'));
        assert_eq!(d.color.as_deref(), Some("#34C759"));
        d.tecla(TeclaPulsada::Retroceso);
        assert_eq!(d.libre.as_ref().map(|l| l.hex.as_str()), Some("#34C75"));
        d.tecla(TeclaPulsada::Caracter('z'));
        assert_eq!(d.libre.as_ref().map(|l| l.hex.as_str()), Some("#34C75"));
    }

    #[test]
    fn hsv_y_hex_son_inversos() {
        for hex in MUESTRAS {
            let (h, s, v) = hex_a_hsv(hex).expect("muestra válida");
            assert_eq!(hsv_a_hex(h, s, v), hex);
        }
    }
}
