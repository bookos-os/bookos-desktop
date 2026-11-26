use bookos_shell::bloqueo::{self, Peticion};
use bookos_shell::carga_sesion;
use bookos_shell::confirmacion::Confirmacion;
use bookos_shell::{Icono, Link, Network, icono, tema};
use iced::font::{Family, Weight};
use iced::widget::{
    Space, Stack, button, column, container, image, mouse_area, row, svg, text, text_input,
};
use iced::{Border, Center, Color, Element, Fill, Font, Left, Padding, Right};

use crate::sistema::{DISTRIBUCIONES, Idioma};
use crate::usuarios::Persona;
use crate::{
    Acceso, CargaSesion, Estado, FaseCarga, Foco, Mensaje, Operacion, Panel, SACUDIDA, TipoEntrada,
    lente,
};

/// Lo que se ve mientras llega el fondo: el mismo casi negro que el compositor
/// pone en los monitores sin imagen durante el bloqueo.
pub const FONDO: Color = Color::from_rgb(0.015, 0.02, 0.03);
/// Ancho de la columna de usuarios, en reposo.
pub const ANCHO_LISTA: f32 = 220.0;
/// Cuánto se adelanta hacia la izquierda la fila bajo el puntero.
const SALIENTE: f32 = 40.0;
const AVATAR_FILA: f32 = 44.0;
const MARGEN: f32 = 28.0;
const BOTON_ESTADO: f32 = 32.0;
const ICONO_ESTADO: f32 = 20.0;
/// Ancho y márgenes de las tarjetas emergentes del escritorio (`emergente::{energia,lista}.rs`).
const ANCHO_PANEL: f32 = 336.0;
const MARGEN_PANEL: f32 = 16.0;
const INTERIOR_PANEL: f32 = ANCHO_PANEL - MARGEN_PANEL * 2.0;
const GRUPO_PANEL: f32 = 6.0;
const BALDOSA_PANEL: f32 = INTERIOR_PANEL - GRUPO_PANEL * 2.0;
const CAPSULA_PAD: f32 = 6.0;

const RAYO_BATERIA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m3.75 13.5 10.5-11.25L12 10.5h8.25L9.75 21.75 12 13.5H3.75Z"/></svg>"##;
const AVISO_BATERIA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z"/></svg>"##;

// Macizos y no de trazo, como los del resto del shell: sobre una foto, un
// trazo de 1,5 se pierde. Wi-Fi y encendido son los `solid` de Heroicons de
// la copia del HIG; el teclado no tiene versión maciza y va con trazo de 2.
pub const WIFI: &str = bloqueo::WIFI;
pub const ENCENDIDO: &str = bloqueo::ENCENDIDO;
pub const TECLADO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="20" height="14" rx="3"/><path d="M6 9.25h.5m3.25 0h.5m3.25 0h.5m3.25 0h.5M6 12.25h.5m3.25 0h.5m3.25 0h.5m3.25 0h.5M8 15.25h8"/></svg>"##;

type Elemento<'a> = Element<'a, Mensaje>;

// 0.1.1
fn blanco(a: f32) -> Color {
    Color { a, ..Color::WHITE }
}

/// Tinta adaptada al tema actual sobre las superficies (`fondo_campo`), igual que `bloqueo.rs`.
fn tinta_campo(a: f32) -> Color {
    tema::alfa(bloqueo::tinta_campo(), a)
}

/// Fondo traslúcido adaptado al tema actual (`bloqueo::fondo_campo`).
fn fondo_campo(a: f32) -> Color {
    tema::alfa(bloqueo::fondo_campo(), a)
}

// 0.1.1
fn peso(weight: Weight) -> Font {
    Font {
        weight,
        ..Font::DEFAULT
    }
}

// 0.1.1
fn alto(valor: f32) -> Space {
    Space::new().height(valor.max(0.0))
}

// 0.1.1
fn pictograma<'a>(icono: &Icono, ancho: f32, alto: f32, color: Option<Color>) -> Elemento<'a> {
    match icono {
        // El alfa va aparte: el teñido de un SVG sustituye el color y no hace
        // caso de su transparencia (una sombra al 35 % salía negra maciza).
        Icono::Svg(dibujo) => svg(dibujo.clone())
            .width(ancho)
            .height(alto)
            .opacity(color.map_or(1.0, |c| c.a))
            .style(move |_, _| svg::Style {
                color: color.map(|c| Color { a: 1.0, ..c }),
            })
            .into(),
        Icono::Raster(foto) => image(foto.clone()).width(ancho).height(alto).into(),
    }
}

fn propio<'a>(nombre: &str, lado: f32, color: Color) -> Elemento<'a> {
    match icono::propio(nombre) {
        Some(icono) => pictograma(&icono, lado, lado, Some(color)),
        None => Space::new().width(lado).height(lado).into(),
    }
}

fn solido<'a>(nombre: &str, lado: f32, color: Color) -> Elemento<'a> {
    match icono::propio_solido(nombre) {
        Some(icono) => pictograma(&icono, lado, lado, Some(color)),
        None => propio(nombre, lado, color),
    }
}

fn encendido_colores() -> (Color, Color) {
    let acento = tema::acento();
    if tema::es_claro() || tema::contraste(Color::WHITE, acento) < 3.0 {
        (acento, tema::sobre_acento())
    } else {
        (Color::WHITE, acento)
    }
}

/// Título de tarjeta de escritorio (`emergente::control::titulo`).
fn titulo_panel<'a>(texto: &'a str, u: f32, alfa: f32) -> Elemento<'a> {
    text(texto)
        .size(17.0 * u)
        .font(peso(Weight::Bold))
        .color(tema::alfa(tema::texto(), alfa))
        .into()
}

/// Grupo gris redondeado dentro de una tarjeta (`emergente::lista::grupo`).
fn grupo_panel<'a>(contenido: Elemento<'a>, u: f32, alfa: f32) -> Elemento<'a> {
    container(contenido)
        .width(INTERIOR_PANEL * u)
        .padding(GRUPO_PANEL * u)
        .style(move |_| container::Style {
            background: Some(tema::alfa(tema::superficie_en_tarjeta(), alfa).into()),
            border: Border {
                radius: (tema::R_TARJETA * u).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Selector segmentado con píldora deslizante (`emergente::energia::selector`).
fn capsula_segmentada<'a>(
    opciones: Vec<(&'a str, Mensaje)>,
    posicion: f32,
    alto_capsula: f32,
    u: f32,
    alfa: f32,
) -> Elemento<'a> {
    let n = opciones.len().max(1);
    let ancho_seg = (INTERIOR_PANEL - CAPSULA_PAD * 2.0) / n as f32;
    let alto_pulgar = alto_capsula - CAPSULA_PAD * 2.0;

    let pista = container(Space::new())
        .width(INTERIOR_PANEL * u)
        .height(alto_capsula * u)
        .style(move |_| container::Style {
            background: Some(tema::alfa(tema::superficie_en_tarjeta(), alfa).into()),
            border: Border {
                radius: (alto_capsula * 0.5 * u).into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let pulgar_color = tema::texto();
    let pulgar = container(Space::new())
        .width(ancho_seg * u)
        .height(alto_pulgar * u)
        .style(move |_| container::Style {
            background: Some(tema::alfa(pulgar_color, alfa).into()),
            border: Border {
                radius: (alto_pulgar * 0.5 * u).into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let pulgar_pos = container(pulgar)
        .width(INTERIOR_PANEL * u)
        .height(alto_capsula * u)
        .padding(Padding {
            left: (CAPSULA_PAD + posicion * ancho_seg).max(0.0) * u,
            top: CAPSULA_PAD * u,
            ..Padding::ZERO
        });

    let tinta_elegido = tema::tinta_sobre(pulgar_color);
    let mut fila = row![];
    for (i, (etiqueta, msg)) in opciones.into_iter().enumerate() {
        let elegido = (1.0 - (posicion - i as f32).abs()).clamp(0.0, 1.0);
        let color = tema::alfa(tema::mezclar(tema::texto2(), tinta_elegido, elegido), alfa);
        let font = peso(if elegido > 0.5 {
            Weight::Semibold
        } else {
            Weight::Medium
        });
        fila = fila.push(
            button(
                container(
                    text(etiqueta)
                        .size(13.0 * u)
                        .font(font)
                        .color(color),
                )
                .width(ancho_seg * u)
                .height(alto_capsula * u)
                .center_x(ancho_seg * u)
                .center_y(alto_capsula * u),
            )
            .width(ancho_seg * u)
            .height(alto_capsula * u)
            .padding(0)
            .on_press(msg)
            .style(|_, _| sin_fondo()),
        );
    }

    Stack::new()
        .push(pista)
        .push(pulgar_pos)
        .push(
            container(fila)
                .width(INTERIOR_PANEL * u)
                .height(alto_capsula * u)
                .padding(Padding {
                    left: CAPSULA_PAD * u,
                    ..Padding::ZERO
                }),
        )
        .into()
}

/// Interruptor deslizante de escritorio (`emergente::lista::interruptor`).
fn interruptor_panel<'a>(avance: f32, u: f32, alfa: f32, msg: Mensaje) -> Elemento<'a> {
    let ancho = 46.0;
    let alto = 28.0;
    let lado = alto - 6.0;
    let bolita = container(Space::new())
        .width(lado * u)
        .height(lado * u)
        .style(move |_| container::Style {
            background: Some(Color {
                a: alfa,
                ..Color::WHITE
            }
            .into()),
            border: Border {
                radius: (lado * 0.5 * u).into(),
                ..Default::default()
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.2 * alfa),
                offset: iced::Vector::new(0.0, 1.0 * u),
                blur_radius: 3.0 * u,
            },
            ..Default::default()
        });
    let recorrido = ((ancho - alto) * avance).max(0.0) * u;
    let pista = container(
        row![Space::new().width(recorrido), bolita].align_y(Center),
    )
    .width(ancho * u)
    .height(alto * u)
    .padding(3.0 * u)
    .style(move |_| container::Style {
        background: Some(
            tema::alfa(
                tema::mezclar(
                    tema::control_apagado(),
                    tema::acento(),
                    avance.clamp(0.0, 1.0),
                ),
                alfa,
            )
            .into(),
        ),
        border: Border {
            radius: (alto * 0.5 * u).into(),
            ..Default::default()
        },
        ..Default::default()
    });
    button(pista)
        .padding(0)
        .on_press(msg)
        .style(|_, _| sin_fondo())
        .into()
}

fn nivel_senal(senal: u8) -> usize {
    match senal {
        67.. => 2,
        34..=66 => 1,
        _ => 0,
    }
}

fn duracion_bateria(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

// 0.1.1
fn sin_fondo() -> button::Style {
    button::Style {
        background: None,
        ..Default::default()
    }
}

/// Un pictograma blanco con la misma sombra que [`rotulo`], por lo mismo.
// 0.1.1
fn con_sombra<'a>(icono: &Icono, lado: f32, alfa: f32, u: f32) -> Elemento<'a> {
    let caida = u.round().max(1.0);
    let capa = |color: Color, arriba: f32| {
        container(pictograma(icono, lado, lado, Some(color))).padding(Padding {
            top: arriba,
            bottom: caida - arriba,
            ..Padding::ZERO
        })
    };
    Stack::new()
        .push(capa(
            Color {
                a: 0.35 * alfa,
                ..Color::BLACK
            },
            caida,
        ))
        .push(capa(blanco(alfa), 0.0))
        .into()
}

/// Texto blanco con una sombra debajo. Va sobre el fondo de pantalla sin
/// píldora que lo separe, y con un fondo claro el blanco solo no se lee.
// 0.1.1
fn rotulo<'a>(texto: String, cuerpo: f32, fuente: Font, alfa: f32) -> Elemento<'a> {
    let capa = move |color: Color| text(texto.clone()).size(cuerpo).font(fuente).color(color);
    let caida = (cuerpo / 16.0).max(1.0);
    Stack::new()
        .push(
            container(capa(Color {
                a: 0.45 * alfa,
                ..Color::BLACK
            }))
            .padding(Padding {
                top: caida,
                left: 0.0,
                right: 0.0,
                bottom: 0.0,
            }),
        )
        .push(container(capa(blanco(alfa))).padding(Padding {
            bottom: caida,
            ..Padding::ZERO
        }))
        .into()
}

// 0.1.1
fn avatar<'a>(
    foto: Option<&image::Handle>,
    sin_foto: Elemento<'a>,
    lado: f32,
    relleno: Color,
    aro: (f32, Color),
    alfa: f32,
) -> Elemento<'a> {
    let dentro = match foto {
        Some(foto) => image(foto.clone())
            .width(lado)
            .height(lado)
            .opacity(alfa)
            .into(),
        None => sin_foto,
    };
    container(dentro)
        .center_x(lado)
        .center_y(lado)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(tema::alfa(relleno, 0.55 * alfa).into()),
            border: Border {
                radius: (lado / 2.0).into(),
                width: aro.0,
                color: tema::alfa(aro.1, aro.1.a * alfa),
            },
            ..Default::default()
        })
        .into()
}

// 0.1.1
fn separador_panel<'a>(ancho: f32, u: f32, alfa: f32) -> Elemento<'a> {
    container(
        container(Space::new().height(u.round()))
            .width(ancho * u)
            .style(move |_| container::Style {
                background: Some(tinta_campo(0.12 * alfa).into()),
                ..Default::default()
            }),
    )
    .center_x(Fill)
    .into()
}

impl Acceso {
    // 0.1.1
    fn persona(&self) -> Option<&Persona> {
        self.personas.iter().find(|p| p.cuenta == self.usuario)
    }

    // 0.1.1
    fn paleta(&self) -> Option<crate::usuarios::Paleta> {
        self.personas.get(self.de)?.apariencia.paleta
    }

    // 0.1.1
    fn acento(&self) -> Color {
        self.paleta().map_or_else(tema::acento, |p| p.acento)
    }

    // 0.1.1
    fn sobre_acento(&self) -> Color {
        self.paleta().map_or(Color::WHITE, |p| p.sobre_acento)
    }

    // 0.1.1
    fn config(&self) -> bookos_shell::ConfigBloqueo {
        self.personas
            .get(self.de)
            .map(|p| p.apariencia.bloqueo)
            .unwrap_or_default()
    }

    // 0.1.1
    fn sacudida(&self) -> f32 {
        let t = self
            .fallo_desde
            .map_or(1.0, |desde| tema::avance(desde.elapsed(), SACUDIDA));
        (t * std::f32::consts::TAU * 3.0).sin() * (1.0 - t) * 11.0 * self.escala
    }

    // 0.1.1
    pub fn vista(&self) -> Elemento<'_> {
        if let Some(carga) = &self.carga_sesion {
            return self.carga(carga);
        }
        let [reloj_avance, entrada, estado_avance] = self.entradas();
        let mut capas = Stack::new().width(Fill).height(Fill);
        if let Some(fondo) = &self.fondo {
            capas = capas.push(column(fondo.losas.iter().map(|fila| {
                row(fila.iter().map(|(losa, ancho, alto)| {
                    image(losa.clone())
                        .width(*ancho)
                        .height(*alto)
                        .filter_method(image::FilterMethod::Nearest)
                        .into()
                }))
                .into()
            })));
        }
        capas = capas.push(self.reloj(reloj_avance)).push(if self.en_lista() {
            self.lista_de_usuarios(entrada)
        } else {
            self.bloque_de_acceso(entrada)
        });
        capas = capas.push(self.fila_de_estado(estado_avance));
        let abierto = self.menu_avance();
        if abierto > 0.001 {
            let panel = self
                .panel_abierto
                .or(self.panel_visible)
                .unwrap_or(Panel::Energia);
            capas = capas
                .push(
                    mouse_area(Space::new().width(Fill).height(Fill))
                        .on_press(Mensaje::AbrirPanel(None)),
                )
                .push(self.panel_emergente(panel, abierto));
        }
        if let Some((peticion, aceptar)) = self.confirmacion {
            capas = capas.push(self.confirmacion_energia(peticion, aceptar));
        }
        let velo = self.velo_entrada();
        if velo > 0.002 {
            capas = capas.push(
                container(Space::new().width(Fill).height(Fill)).style(move |_| container::Style {
                    background: Some(Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: velo,
                    }
                    .into()),
                    ..Default::default()
                }),
            );
        }
        container(capas)
            .style(|_| container::Style {
                background: Some(FONDO.into()),
                ..Default::default()
            })
            .into()
    }

    fn carga<'a>(&'a self, carga: &CargaSesion) -> Elemento<'a> {
        let u = self.escala;
        let lado = carga_sesion::LOGO as f32 * u;
        let ancho = carga_sesion::ANCHO_BARRA as f32 * u;
        let alto_barra = carga_sesion::ALTO_BARRA as f32 * u;
        let reducidos = tema::efectos_reducidos();
        let (progreso, pulso) = if reducidos {
            (carga_sesion::PROGRESO_REDUCIDO, 1.0)
        } else {
            // Solo pulsa la tinta. La barra mantiene el mismo tramo que la primera
            // imagen de KMS y no representa un porcentaje de autenticación.
            let periodo = 4.0 * tema::D_TEMA.as_secs_f32();
            let t = carga.desde.elapsed().as_secs_f32() * std::f32::consts::TAU / periodo;
            (carga_sesion::PROGRESO_INICIAL, 0.88 + 0.12 * t.cos())
        };
        let pildora = |ancho, [r, g, b, a]: [u8; 4], pulso| {
            container(Space::new())
                .width(ancho)
                .height(alto_barra)
                .style(move |_| container::Style {
                    background: Some(
                        Color::from_rgba8(r, g, b, f32::from(a) / 255.0 * pulso).into(),
                    ),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                })
        };
        let barra = Stack::new()
            .width(ancho)
            .height(alto_barra)
            .push(pildora(ancho, carga_sesion::PISTA, 1.0))
            .push(pildora(ancho * progreso, carga_sesion::RELLENO, pulso));
        let estado = match &self.estado {
            Estado::Ocupado(Operacion::Iniciar) => {
                self.idioma.t("Iniciando sesión…", "Signing in…")
            }
            _ if carga.fase == FaseCarga::ArranquePendiente => {
                self.idioma.t("Iniciando sesión…", "Signing in…")
            }
            Estado::Aviso(aviso) => aviso.as_str(),
            _ => self.idioma.t("Comprobando el acceso…", "Checking sign-in…"),
        };
        let separacion = carga_sesion::SEPARACION as f32 * u;
        let desfase = carga_sesion::DESFASE_Y as f32 * u;
        let emblema = column![
            bookos_shell::marca::ver(lado).map(|()| Mensaje::Fotograma),
            alto(separacion),
            barra,
        ]
        .align_x(Center)
        .padding(Padding::default().bottom(2.0 * desfase));
        // Los avisos de PAM pueden ocupar varias líneas sin desplazar el logo
        // que recogerá el compositor al relevar al greeter.
        let y_texto =
            (self.pantalla.height + lado + separacion + alto_barra) / 2.0 - desfase + 16.0 * u;
        let texto = container(text(estado).size(tema::T_CUERPO * u).color(Color::WHITE))
            .center_x(Fill)
            .padding([0.0, 16.0 * u]);
        let capas = Stack::new()
            .push(container(emblema).center(Fill))
            .push(column![alto(y_texto), texto].width(Fill));
        container(capas)
            .style(|_| container::Style {
                background: Some(Color::BLACK.into()),
                ..Default::default()
            })
            .into()
    }

    /// Hora y fecha, en el mismo orden, blanco y entrada escalonada que el bloqueo.
    // modificado en la 0.2.0 -- antes la fecha iba encima y con el tono de la paleta.
    // 0.1.1
    fn reloj(&self, alfa: f32) -> Elemento<'_> {
        let (u, config) = (self.escala, self.config());
        let familia = Family::Name(config.reloj_fuente.familia());
        let tinta = blanco(alfa);
        let mut reloj = column![
            alto(self.pantalla.height * config.reloj_y + (1.0 - alfa) * 22.0 * u),
            text(&self.hora)
                .size(config.reloj_tamano * u)
                .font(Font {
                    family: familia,
                    weight: bloqueo::grosor(config.reloj_grosor),
                    ..Font::DEFAULT
                })
                .color(tinta)
                .width(Fill)
                .align_x(Center),
        ];
        if config.fecha {
            reloj = reloj.push(
                text(&self.fecha)
                    .size(22.0 * u)
                    .font(Font {
                        family: familia,
                        weight: Weight::Medium,
                        ..Font::DEFAULT
                    })
                    .color(blanco(0.75 * alfa))
                    .width(Fill)
                    .align_x(Center),
            );
        }
        reloj.into()
    }

    // 0.1.1
    fn boton_estado<'a>(
        &'a self,
        contenido: Elemento<'a>,
        cual: Panel,
        circular: bool,
    ) -> Elemento<'a> {
        let u = self.escala;
        let alto_boton = BOTON_ESTADO * u;
        let activo = self.panel_abierto == Some(cual) && self.menu.destino() > 0.0;
        let caja = if circular {
            container(contenido)
                .center_x(alto_boton)
                .center_y(alto_boton)
        } else {
            container(contenido)
                .padding([0.0, 9.0 * u])
                .center_y(alto_boton)
        };
        button(caja)
            .padding(0)
            .on_press(Mensaje::AbrirPanel(Some(cual)))
            .style(move |_, estado| button::Style {
                background: (activo || estado != button::Status::Active)
                    .then(|| blanco(0.18).into()),
                border: Border {
                    radius: (alto_boton / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.1
    fn fila_de_estado(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let lado = ICONO_ESTADO * u;
        let etiqueta = |texto: &str| {
            rotulo(
                texto.to_string(),
                (13.0 * u).round(),
                peso(Weight::Semibold),
                alfa,
            )
        };

        let mut fila = row![].spacing(6.0 * u).align_y(Center);
        if let Some(distribucion) = &self.teclado {
            fila = fila.push(
                self.boton_estado(
                    row![
                        etiqueta(distribucion),
                        con_sombra(&self.tecla, lado, alfa, u)
                    ]
                    .spacing(6.0 * u)
                    .align_y(Center)
                    .into(),
                    Panel::Idioma,
                    false,
                ),
            );
        }
        if let Some((icono, (ancho, alto), cifra, tinta)) = &self.bateria {
            let cifra: Elemento<'_> = if *tinta == Color::WHITE {
                etiqueta(cifra)
            } else {
                text(cifra)
                    .size((13.0 * u).round())
                    .font(peso(Weight::Semibold))
                    .color(tema::alfa(*tinta, alfa))
                    .into()
            };
            fila = fila.push(
                self.boton_estado(
                    row![pictograma(icono, ancho * u, alto * u, None), cifra]
                        .spacing(5.0 * u)
                        .align_y(Center)
                        .into(),
                    Panel::Bateria,
                    false,
                ),
            );
        }
        let icono_red: Elemento<'_> = match self.red {
            Some(Network {
                kind: Link::Cable,
                up: true,
            }) => propio("cable", lado, blanco(alfa)),
            // Sin red son los mismos arcos apagados: un icono tachado más en
            // la fila llama más la atención que el dato que da.
            Some(Network { up: true, .. }) => con_sombra(&self.wifi, lado, alfa, u),
            Some(Network { up: false, .. }) | None => con_sombra(&self.wifi, lado, 0.35 * alfa, u),
        };
        fila = fila.push(self.boton_estado(icono_red, Panel::Red, true));
        fila = fila.push(self.boton_estado(
            con_sombra(&self.encendido, lado, alfa, u),
            Panel::Energia,
            true,
        ));
        container(fila)
            .width(Fill)
            .align_x(Right)
            .padding((MARGEN - (BOTON_ESTADO - ICONO_ESTADO) / 2.0) * u)
            .into()
    }

    // 0.1.1
    fn panel_emergente(&self, panel: Panel, avance: f32) -> Elemento<'_> {
        let u = self.escala;
        let alfa = avance.clamp(0.0, 1.0);
        let escritorio = !matches!(panel, Panel::Energia);
        let (contenido, ancho) = match panel {
            Panel::Energia => (self.menu_de_energia(alfa), bloqueo::ANCHO_MENU),
            Panel::Idioma => (self.menu_de_idioma(alfa), ANCHO_PANEL),
            Panel::Bateria => (self.menu_de_bateria(alfa), ANCHO_PANEL),
            Panel::Red => (self.menu_de_red(alfa), ANCHO_PANEL),
        };
        let borde = tema::borde();
        let color_borde = tema::alfa(borde, borde.a * alfa);
        let tarjeta = container(contenido)
            .width(ancho * u)
            .padding(if escritorio { MARGEN_PANEL * u } else { 0.0 })
            .clip(true)
            .style(move |_| {
                if escritorio {
                    container::Style {
                        background: Some(tema::alfa(tema::card(), alfa).into()),
                        border: Border {
                            radius: (tema::R_POPOVER * u).into(),
                            width: u.round().max(1.0),
                            color: color_borde,
                        },
                        ..Default::default()
                    }
                } else {
                    container::Style {
                        background: Some(fondo_campo(0.94 * alfa).into()),
                        border: Border {
                            radius: (tema::R_TARJETA * u).into(),
                            width: if tema::es_claro() { u.round() } else { 0.0 },
                            color: color_borde,
                        },
                        ..Default::default()
                    }
                }
            });
        // El clic dentro de la tarjeta no debe cerrar el emergente.
        let protegida = mouse_area(tarjeta).on_press(Mensaje::Fotograma);
        container(protegida)
            .width(Fill)
            .align_x(Right)
            .padding(Padding {
                top: ((MARGEN + BOTON_ESTADO + 6.0 - (1.0 - avance) * 18.0) * u).max(0.0),
                right: (MARGEN - (BOTON_ESTADO - ICONO_ESTADO) / 2.0) * u,
                ..Padding::ZERO
            })
            .into()
    }

    // 0.1.1
    fn menu_de_idioma(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let sub = tema::alfa(tema::texto2(), alfa);
        let texto_principal = tema::alfa(tema::texto(), alfa);
        let (disco_activo, glifo_activo) = encendido_colores();

        let cabecera = container(
            row![
                titulo_panel(self.idioma.t("Idioma y teclado", "Language & keyboard"), u, alfa),
                Space::new().width(Fill),
                container(
                    text("Alt + Shift")
                        .size(11.0 * u)
                        .font(peso(Weight::Semibold))
                        .color(sub),
                )
                .height(20.0 * u)
                .center_y(20.0 * u)
                .padding([0.0, 8.0 * u])
                .style(move |_| container::Style {
                    background: Some(tema::alfa(tema::superficie_en_tarjeta(), alfa).into()),
                    border: Border {
                        radius: (tema::R_CHIP * u).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            ]
            .align_y(Center),
        )
        .width(INTERIOR_PANEL * u)
        .height(32.0 * u)
        .center_y(32.0 * u)
        .padding([0.0, 4.0 * u]);

        let selector_idioma = capsula_segmentada(
            Idioma::TODOS
                .iter()
                .map(|idioma| (idioma.etiqueta(), Mensaje::ElegirIdioma(*idioma)))
                .collect(),
            self.idioma_pulgar.valor(),
            46.0,
            u,
            alfa,
        );

        let rotulo_teclado = container(
            text(self.idioma.t("Distribución de teclado", "Keyboard layout"))
                .size(12.0 * u)
                .font(peso(Weight::Medium))
                .color(sub),
        )
        .padding([0.0, 4.0 * u])
        .height(16.0 * u)
        .center_y(16.0 * u);

        let mut filas = column![].spacing(6.0 * u);
        for (codigo, nombre_es, nombre_en) in DISTRIBUCIONES {
            let activo = self.teclado_codigo == *codigo;
            let nombre = self.idioma.t(nombre_es, nombre_en);
            let disco = if activo { disco_activo } else { tema::elevado() };
            let badge: Elemento<'_> = if activo {
                pictograma(
                    &self.comprobado,
                    14.0 * u,
                    14.0 * u,
                    Some(tema::alfa(glifo_activo, alfa)),
                )
            } else {
                text(codigo.to_uppercase())
                    .size(10.0 * u)
                    .font(peso(Weight::Bold))
                    .color(sub)
                    .into()
            };
            let icono = container(badge)
                .width(32.0 * u)
                .height(32.0 * u)
                .center_x(32.0 * u)
                .center_y(32.0 * u)
                .style(move |_| container::Style {
                    background: Some(tema::alfa(disco, alfa).into()),
                    border: Border {
                        radius: (16.0 * u).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            let fila = row![
                icono,
                Space::new().width(10.0 * u),
                text(nombre)
                    .size(14.0 * u)
                    .font(peso(if activo { Weight::Semibold } else { Weight::Medium }))
                    .color(texto_principal),
                Space::new().width(Fill),
                text(codigo.to_uppercase())
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(if activo {
                        tema::alfa(tema::acento_en(tema::superficie_en_tarjeta()), alfa)
                    } else {
                        sub
                    }),
            ]
            .align_y(Center);
            filas = filas.push(
                button(container(fila).center_y(42.0 * u))
                    .width(BALDOSA_PANEL * u)
                    .height(42.0 * u)
                    .padding(Padding {
                        left: 6.0 * u,
                        right: 12.0 * u,
                        ..Padding::ZERO
                    })
                    .on_press(Mensaje::ElegirTeclado((*codigo).to_string()))
                    .style(move |_, estado| {
                        let hover = estado != button::Status::Active;
                        let fondo = if hover {
                            tema::elevado()
                        } else {
                            tema::superficie_en_tarjeta()
                        };
                        button::Style {
                            background: Some(tema::alfa(fondo, alfa).into()),
                            border: Border {
                                radius: (tema::R_CONTROL * u).into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }),
            );
        }

        column![
            cabecera,
            alto(12.0 * u),
            selector_idioma,
            alto(18.0 * u),
            rotulo_teclado,
            alto(8.0 * u),
            grupo_panel(filas.into(), u, alfa),
        ]
        .into()
    }

    // 0.1.1
    // modificado en la 0.2.1 -- diseño idéntico al popover de batería del escritorio (`emergente::energia.rs`)
    fn menu_de_bateria(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let detalle = &self.bateria_detalle;
        let texto_principal = tema::alfa(tema::texto(), alfa);
        let sub = tema::alfa(tema::texto2(), alfa);
        let es_es = matches!(self.idioma, Idioma::Es);

        let color_perfil = match detalle.perfil.as_str() {
            "power-saver" => tema::perfil_ahorro(),
            "balanced" => tema::perfil_equilibrado(),
            "performance" => tema::perfil_rendimiento(),
            _ => tema::texto2(),
        };

        let baja = detalle
            .dato
            .as_ref()
            .is_some_and(|b| !b.charging && !b.plugged && b.percent <= 15);
        let pastilla = match &detalle.dato {
            None => Some((
                self.idioma.t("Corriente", "AC power"),
                RAYO_BATERIA,
                tema::verde(),
            )),
            Some(b) if b.charging => Some((
                self.idioma.t("Cargando", "Charging"),
                RAYO_BATERIA,
                tema::verde(),
            )),
            Some(b) if b.plugged => Some((
                self.idioma.t("Enchufado", "Plugged in"),
                RAYO_BATERIA,
                tema::verde(),
            )),
            Some(_) if baja => Some((
                self.idioma.t("Batería baja", "Low battery"),
                AVISO_BATERIA,
                tema::rojo(),
            )),
            Some(_) => None,
        };

        let mut titulo_fila = row![
            text(self.idioma.t("Batería", "Battery"))
                .size(15.0 * u)
                .font(peso(Weight::Semibold))
                .color(texto_principal),
        ]
        .spacing(8.0 * u)
        .height(22.0 * u)
        .align_y(Center);
        if let Some((rotulo, svg_icono, color_icono)) = pastilla {
            let ic = icono::desde_svg(svg_icono);
            titulo_fila = titulo_fila.push(
                container(
                    row![
                        pictograma(&ic, 12.0 * u, 12.0 * u, Some(tema::alfa(color_icono, alfa))),
                        text(rotulo)
                            .size(11.0 * u)
                            .font(peso(Weight::Semibold))
                            .color(texto_principal),
                    ]
                    .spacing(4.0 * u)
                    .align_y(Center),
                )
                .height(18.0 * u)
                .center_y(18.0 * u)
                .padding([0.0, 8.0 * u])
                .style(move |_| container::Style {
                    background: Some(tema::alfa(tema::superficie_en_tarjeta(), alfa).into()),
                    border: Border {
                        radius: (tema::R_CHIP * u).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }

        let estado_txt: String = match &detalle.dato {
            None => self
                .idioma
                .t(
                    "Este equipo funciona solo con corriente",
                    "This computer runs on AC power only",
                )
                .into(),
            Some(b) if b.charging => match (b.minutes, detalle.tope) {
                (Some(m), Some(tope)) if m > 0 => {
                    let pct_tope = if es_es {
                        format!("{tope} %")
                    } else {
                        format!("{tope}%")
                    };
                    if es_es {
                        format!("Al {pct_tope} en {}", duracion_bateria(m))
                    } else {
                        format!("{pct_tope} in {}", duracion_bateria(m))
                    }
                }
                (Some(m), None) if m > 0 => {
                    if es_es {
                        format!("Llena en {}", duracion_bateria(m))
                    } else {
                        format!("Full in {}", duracion_bateria(m))
                    }
                }
                _ => self
                    .idioma
                    .t("Cargando la batería", "Charging the battery")
                    .into(),
            },
            Some(b) if b.plugged && detalle.tope.is_some_and(|tope| b.percent + 1 >= tope) => self
                .idioma
                .t(
                    "En pausa para cuidar la batería",
                    "Paused to protect the battery",
                )
                .into(),
            Some(b) if b.plugged && b.percent >= 99 => {
                self.idioma.t("Batería cargada", "Fully charged").into()
            }
            Some(b) if b.plugged => self
                .idioma
                .t("Enchufado, sin cargar", "Plugged in, not charging")
                .into(),
            Some(b) => match b.minutes {
                Some(m) if m > 0 => {
                    if es_es {
                        format!("{} restantes", duracion_bateria(m))
                    } else {
                        format!("{} left", duracion_bateria(m))
                    }
                }
                _ => self.idioma.t("Usando la batería", "On battery").into(),
            },
        };

        let nivel = match &detalle.dato {
            Some(b) => {
                let pct = b.percent.min(100);
                if es_es {
                    format!("{pct} %")
                } else {
                    format!("{pct}%")
                }
            }
            None => self.idioma.t("Sin batería", "No battery").into(),
        };
        let color_nivel = if baja {
            tema::alfa(tema::rojo(), alfa)
        } else {
            texto_principal
        };

        let cabecera_izquierda = column![
            titulo_fila,
            alto(4.0 * u),
            container(
                text(bookos_shell::recortar_texto(
                    &estado_txt,
                    INTERIOR_PANEL - 100.0,
                    12.0,
                ))
                .size(12.0 * u)
                .color(sub),
            )
            .height(16.0 * u)
            .center_y(16.0 * u),
        ];
        let mut cabecera = column![
            row![
                cabecera_izquierda,
                Space::new().width(Fill),
                container(
                    text(nivel)
                        .size(34.0 * u)
                        .font(peso(Weight::ExtraBold))
                        .color(color_nivel),
                )
                .height(42.0 * u)
                .center_y(42.0 * u),
            ]
            .height(42.0 * u),
        ];

        if let Some(bat) = &detalle.dato {
            let pct = bat.percent.min(100);
            let ancho_barra = INTERIOR_PANEL * u;
            let bar_h = 8.0 * u;
            let marca_h = bar_h + 6.0 * u;
            let lleno = (ancho_barra * (pct as f32 / 100.0)).max(bar_h);
            let color_barra = if baja {
                tema::rojo()
            } else if bat.charging || bat.plugged {
                tema::verde()
            } else {
                color_perfil
            };
            let pista = container(
                container(Space::new())
                    .width(lleno)
                    .height(bar_h)
                    .style(move |_| container::Style {
                        background: Some(tema::alfa(color_barra, alfa).into()),
                        border: Border {
                            radius: (tema::R_PILL * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
            .width(ancho_barra)
            .height(bar_h)
            .style(move |_| container::Style {
                background: Some(tema::alfa(tema::control_apagado(), alfa).into()),
                border: Border {
                    radius: (tema::R_PILL * u).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            let mut capas = Stack::new().push(
                container(pista)
                    .height(marca_h)
                    .padding([(marca_h - bar_h) * 0.5, 0.0]),
            );
            if bat.plugged
                && let Some(tope) = detalle.tope
            {
                let x = (ancho_barra * (tope.min(100) as f32 / 100.0) - u).max(0.0);
                capas = capas.push(row![
                    Space::new().width(x),
                    container(Space::new())
                        .width(2.0 * u)
                        .height(marca_h)
                        .style(move |_| container::Style {
                            background: Some(texto_principal.into()),
                            border: Border {
                                radius: u.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                ]);
            }
            cabecera = cabecera
                .push(alto(12.0 * u))
                .push(container(capas).height(marca_h));
        }

        let perfiles: Vec<(&str, &str, &str)> = [
            (
                "power-saver",
                self.idioma.t("Ahorro", "Saver"),
                self.idioma.t(
                    "Más autonomía, menos potencia",
                    "Longer battery life, less power",
                ),
            ),
            (
                "balanced",
                self.idioma.t("Equilibrado", "Balanced"),
                self.idioma
                    .t("Potencia según lo que hagas", "Power adapts to what you do"),
            ),
            (
                "performance",
                self.idioma.t("Rendimiento", "Performance"),
                self.idioma
                    .t("Máxima potencia, más consumo", "Maximum power, higher usage"),
            ),
        ]
        .into_iter()
        .filter(|(clave, _, _)| {
            detalle.perfiles_disponibles.is_empty()
                || detalle.perfiles_disponibles.iter().any(|p| p == clave)
        })
        .collect();

        let mut col = column![cabecera];
        if !perfiles.is_empty() {
            let detalle_activo = perfiles
                .iter()
                .find(|(clave, _, _)| *clave == detalle.perfil)
                .map_or("", |(_, _, d)| *d);
            let selector = capsula_segmentada(
                perfiles
                    .iter()
                    .map(|(clave, corto, _)| (*corto, Mensaje::PerfilEnergia((*clave).into())))
                    .collect(),
                self.pulgar.valor(),
                54.0,
                u,
                alfa,
            );
            col = col
                .push(alto(18.0 * u))
                .push(
                    container(
                        text(self.idioma.t("Modo de energía", "Power mode"))
                            .size(12.0 * u)
                            .font(peso(Weight::Medium))
                            .color(sub),
                    )
                    .padding([0.0, 4.0 * u])
                    .height(16.0 * u)
                    .center_y(16.0 * u),
                )
                .push(alto(8.0 * u))
                .push(selector)
                .push(alto(10.0 * u))
                .push(
                    container(text(detalle_activo).size(12.0 * u).color(sub))
                        .padding([0.0, 4.0 * u])
                        .height(17.0 * u)
                        .center_y(17.0 * u),
                );
        }
        col.into()
    }

    // 0.1.1
    // modificado en la 0.2.1 -- tarjetas y filas del sistema de diseño (`emergente::{red,lista}.rs`)
    fn menu_de_red(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let wifi = &self.wifi_estado;
        let acento = tema::acento();
        let sobre = tema::sobre_acento();
        let texto_principal = tema::alfa(tema::texto(), alfa);
        let sub = tema::alfa(tema::texto2(), alfa);
        let (disco_activo, glifo_activo) = encendido_colores();

        let mut fila_cabecera = row![titulo_panel("Wi-Fi", u, alfa), Space::new().width(Fill)]
            .spacing(8.0 * u)
            .align_y(Center);
        if wifi.encendida {
            let recargar = button(
                container(propio("reiniciar", 15.0 * u, sub))
                    .center_x(26.0 * u)
                    .center_y(26.0 * u),
            )
            .padding(0)
            .on_press(Mensaje::WifiRefrescar)
            .style(move |_, estado| button::Style {
                background: (estado != button::Status::Active)
                    .then(|| tema::alfa(tema::superficie_en_tarjeta(), alfa).into()),
                border: Border {
                    radius: (13.0 * u).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            fila_cabecera = fila_cabecera.push(recargar);
        }
        fila_cabecera = fila_cabecera.push(interruptor_panel(
            self.wifi_switch.valor(),
            u,
            alfa,
            Mensaje::WifiAlternar(!wifi.encendida),
        ));

        let cabecera = container(fila_cabecera)
            .width(INTERIOR_PANEL * u)
            .height(32.0 * u)
            .center_y(32.0 * u)
            .padding([0.0, 4.0 * u]);

        let vacia = |mensaje: String| -> Elemento<'_> {
            container(text(mensaje).size(13.0 * u).color(sub))
                .width(BALDOSA_PANEL * u)
                .height(48.0 * u)
                .center_x(BALDOSA_PANEL * u)
                .center_y(48.0 * u)
                .into()
        };

        let mut cuerpo = column![].spacing(6.0 * u);
        if wifi.cable {
            cuerpo = cuerpo.push(
                container(
                    row![
                        propio("cable", 16.0 * u, tema::alfa(tema::verde(), alfa)),
                        text(
                            self.idioma
                                .t("Conectado por cable Ethernet", "Ethernet cable connected"),
                        )
                        .size(12.0 * u)
                        .font(peso(Weight::Medium))
                        .color(texto_principal),
                    ]
                    .spacing(8.0 * u)
                    .align_y(Center),
                )
                .width(BALDOSA_PANEL * u)
                .padding([6.0 * u, 10.0 * u])
                .style(move |_| container::Style {
                    background: Some(tema::alfa(tema::elevado(), alfa).into()),
                    border: Border {
                        radius: (tema::R_CONTROL * u).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        if let Some(err) = self.wifi_error.as_deref().or(wifi.error.as_deref()) {
            cuerpo = cuerpo.push(
                container(
                    text(err)
                        .size(12.0 * u)
                        .color(tema::alfa(tema::rojo(), alfa)),
                )
                .width(BALDOSA_PANEL * u)
                .padding([4.0 * u, 8.0 * u]),
            );
        }

        let clave_card = |ssid: &str| -> Elemento<'_> {
            let listo = self.wifi_clave.chars().count() >= 8;
            let estilo_input = move |_: &iced::Theme, _| text_input::Style {
                background: tema::alfa(tema::elevado(), alfa).into(),
                border: Border {
                    radius: (tema::R_CONTROL * u).into(),
                    width: 1.5 * u,
                    color: tema::alfa(acento, alfa),
                },
                icon: texto_principal,
                placeholder: sub,
                value: texto_principal,
                selection: tema::alfa(acento, 0.35 * alfa),
            };
            column![
                text_input(
                    self.idioma
                        .t("Contraseña (mín. 8 caracteres)", "Password (min. 8 chars)"),
                    &self.wifi_clave,
                )
                .id("wifi_clave")
                .secure(true)
                .size(14.0 * u)
                .padding([10.0 * u, 12.0 * u])
                .on_input(Mensaje::WifiClave)
                .on_submit(Mensaje::WifiConectar)
                .style(estilo_input),
                row![
                    button(
                        container(
                            text(self.idioma.t("Cancelar", "Cancel"))
                                .size(13.0 * u)
                                .font(peso(Weight::Semibold))
                                .color(texto_principal),
                        )
                        .center_x(Fill)
                        .center_y(32.0 * u),
                    )
                    .width(Fill)
                    .padding(0)
                    .on_press(Mensaje::WifiCancelarClave)
                    .style(move |_, estado| button::Style {
                        background: Some(
                            tema::alfa(
                                tema::mezclar(
                                    tema::elevado(),
                                    tema::tinta(),
                                    if estado != button::Status::Active { 0.06 } else { 0.0 },
                                ),
                                alfa,
                            )
                            .into(),
                        ),
                        border: Border {
                            radius: (tema::R_BOTON_PEQUENO * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    button(
                        container(
                            text(self.idioma.t("Conectar", "Connect"))
                                .size(13.0 * u)
                                .font(peso(Weight::Semibold))
                                .color(if listo {
                                    tema::alfa(sobre, alfa)
                                } else {
                                    sub
                                }),
                        )
                        .center_x(Fill)
                        .center_y(32.0 * u),
                    )
                    .width(Fill)
                    .padding(0)
                    .on_press_maybe(listo.then_some(Mensaje::WifiConectar))
                    .style(move |_, _| button::Style {
                        background: Some(if listo {
                            tema::alfa(acento, alfa).into()
                        } else {
                            tema::alfa(tema::elevado(), alfa).into()
                        }),
                        border: Border {
                            radius: (tema::R_BOTON_PEQUENO * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ]
                .spacing(6.0 * u),
                text(format!(
                    "{} «{ssid}»",
                    self.idioma.t("Red seleccionada:", "Selected network:")
                ))
                .size(11.0 * u)
                .color(sub),
            ]
            .spacing(6.0 * u)
            .width(BALDOSA_PANEL * u)
            .padding(Padding {
                top: 2.0 * u,
                bottom: 4.0 * u,
                ..Padding::ZERO
            })
            .into()
        };

        let seccion_wifi = |titulo: &str| -> Elemento<'_> {
            container(
                text(titulo.to_string())
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(sub),
            )
            .width(BALDOSA_PANEL * u)
            .height(26.0 * u)
            .padding(Padding {
                left: 6.0 * u,
                ..Padding::ZERO
            })
            .center_y(26.0 * u)
            .into()
        };

        if self.wifi_cargando {
            cuerpo = cuerpo.push(vacia(
                self.idioma
                    .t("Buscando redes…", "Scanning networks…")
                    .into(),
            ));
        } else if !wifi.encendida {
            cuerpo = cuerpo.push(vacia(
                self.idioma.t("Wi-Fi apagado", "Wi-Fi is off").into(),
            ));
        } else if wifi.redes.is_empty() {
            cuerpo = cuerpo.push(vacia(
                self.idioma
                    .t("No hay redes a la vista", "No networks in range")
                    .into(),
            ));
        } else {
            let ancho_nombre = BALDOSA_PANEL - 12.0 - 36.0 - 12.0 - 44.0;
            let mut en_conocidas = false;
            let mut en_otras = false;
            let mut clave_mostrada = false;

            for (i, red) in wifi.redes.iter().enumerate() {
                if !red.activa {
                    if red.guardada && !en_conocidas {
                        en_conocidas = true;
                        cuerpo = cuerpo.push(seccion_wifi(
                            self.idioma.t("Redes conocidas", "Known networks"),
                        ));
                    } else if !red.guardada && !en_otras {
                        en_otras = true;
                        cuerpo = cuerpo.push(seccion_wifi(
                            self.idioma.t("Otras redes", "Other networks"),
                        ));
                    }
                }

                let (disco, glifo, estado_color) = if red.activa {
                    (
                        disco_activo,
                        glifo_activo,
                        tema::alfa(tema::acento_en(tema::superficie_en_tarjeta()), alfa),
                    )
                } else {
                    (tema::elevado(), tema::texto(), sub)
                };
                let nombre_icono = ["wifi-1", "wifi-2", "wifi"][nivel_senal(red.senal)];
                let dibujo = container(solido(nombre_icono, 18.0 * u, tema::alfa(glifo, alfa)))
                    .width(36.0 * u)
                    .height(36.0 * u)
                    .center_x(36.0 * u)
                    .center_y(36.0 * u)
                    .style(move |_| container::Style {
                        background: Some(tema::alfa(disco, alfa).into()),
                        border: Border {
                            radius: (18.0 * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    });

                let mut textos = column![
                    text(bookos_shell::recortar_texto(&red.ssid, ancho_nombre, 14.0))
                        .size(14.0 * u)
                        .font(peso(if red.activa {
                            Weight::Semibold
                        } else {
                            Weight::Medium
                        }))
                        .color(texto_principal),
                ];
                if red.activa {
                    let calidad = match nivel_senal(red.senal) {
                        2 => self.idioma.t("Excelente", "Excellent"),
                        1 => self.idioma.t("Buena", "Good"),
                        _ => self.idioma.t("Débil", "Weak"),
                    };
                    textos = textos.push(
                        text(format!(
                            "{} · {calidad}",
                            self.idioma.t("Conectado", "Connected"),
                        ))
                        .size(11.0 * u)
                        .color(estado_color),
                    );
                }

                let mut fila = row![dibujo, Space::new().width(12.0 * u), textos].align_y(Center);
                if red.segura && !red.activa {
                    fila = fila
                        .push(Space::new().width(Fill))
                        .push(solido("bloquear", 13.0 * u, sub));
                }

                cuerpo = cuerpo.push(
                    button(container(fila).center_y(48.0 * u))
                        .width(BALDOSA_PANEL * u)
                        .height(48.0 * u)
                        .padding(Padding {
                            left: 6.0 * u,
                            right: 12.0 * u,
                            ..Padding::ZERO
                        })
                        .on_press(Mensaje::WifiElegir(i))
                        .style(move |_, estado| {
                            let fondo = if estado != button::Status::Active {
                                tema::elevado()
                            } else {
                                tema::superficie_en_tarjeta()
                            };
                            button::Style {
                                background: Some(tema::alfa(fondo, alfa).into()),
                                border: Border {
                                    radius: (tema::R_CONTROL * u).into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }
                        }),
                );

                if self.wifi_conectar_a.as_deref() == Some(red.ssid.as_str()) {
                    clave_mostrada = true;
                    cuerpo = cuerpo.push(clave_card(&red.ssid));
                }
            }

            if !clave_mostrada && let Some(ssid) = &self.wifi_conectar_a {
                cuerpo = cuerpo.push(clave_card(ssid));
            }

            if wifi.ssid_actual.is_some() {
                let color_desconectar = tema::alfa(tema::rojo_en(tema::elevado()), alfa);
                let pie = button(
                    container(
                        text(self.idioma.t("Desconectar", "Disconnect"))
                            .size(14.0 * u)
                            .font(peso(Weight::Semibold))
                            .color(color_desconectar),
                    )
                    .width(BALDOSA_PANEL * u)
                    .height(36.0 * u)
                    .center_x(BALDOSA_PANEL * u)
                    .center_y(36.0 * u),
                )
                .width(BALDOSA_PANEL * u)
                .height(36.0 * u)
                .padding(0)
                .on_press(Mensaje::WifiDesconectar)
                .style(move |_, estado| {
                    let s = if estado != button::Status::Active {
                        1.0
                    } else {
                        0.0
                    };
                    button::Style {
                        background: Some(
                            tema::alfa(tema::mezclar(tema::elevado(), tema::tinta(), 0.05 * s), alfa)
                                .into(),
                        ),
                        border: Border {
                            radius: (tema::R_CONTROL * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                });
                cuerpo = cuerpo
                    .push(alto(4.5 * u))
                    .push(pie);
            }
        }

        column![cabecera, alto(12.0 * u), grupo_panel(cuerpo.into(), u, alfa)].into()
    }

    // 0.1.1
    // modificado en la 0.2.0 -- glifos y peso Medium emparejados con el bloqueo (`bloqueo.rs`)
    fn menu_de_energia(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let tinta = tinta_campo(alfa);
        let entradas: Vec<_> = bloqueo::entradas()
            .iter()
            // Sin sesión abierta, cerrarla o cambiar de usuario no significan
            // nada: la lista de cuentas ya está en pantalla.
            .filter(|(_, _, peticion)| {
                !matches!(peticion, Peticion::CerrarSesion | Peticion::CambiarUsuario)
            })
            .collect();
        let mut filas = column![];
        for (i, (etiqueta, icono, peticion)) in entradas.iter().enumerate() {
            let etiqueta_traducida = match peticion {
                Peticion::Apagar => self.idioma.t("Apagar", "Shut Down"),
                Peticion::Reiniciar => self.idioma.t("Reiniciar", "Restart"),
                Peticion::Suspender => self.idioma.t("Suspender", "Sleep"),
                Peticion::Hibernar => self.idioma.t("Hibernar", "Hibernate"),
                Peticion::CerrarSesion | Peticion::CambiarUsuario => *etiqueta,
            };
            let fila = row![
                propio(icono, 20.0 * u, tinta),
                text(etiqueta_traducida)
                    .size(15.0 * u)
                    .font(peso(Weight::Medium))
                    .color(tinta),
            ]
            .spacing(12.0 * u)
            .align_y(Center);
            filas = filas.push(
                button(container(fila).center_y(Fill))
                    .padding([0.0, 18.0 * u])
                    .width(Fill)
                    .height((bloqueo::ALTO_ENTRADA - 1.0) * u)
                    .on_press(Mensaje::Energia(*peticion))
                    .style(move |_, estado| button::Style {
                        background: (estado != button::Status::Active)
                            .then(|| tinta_campo(0.10 * alfa).into()),
                        ..Default::default()
                    }),
            );
            if i + 1 < entradas.len() {
                filas = filas.push(separador_panel(bloqueo::ANCHO_MENU - 36.0, u, alfa));
            }
        }
        filas.into()
    }

    /// Diálogo de confirmación de energía (Apagar / Reiniciar), siguiendo `confirmacion.rs`.
    fn confirmacion_energia(&self, peticion: Peticion, aceptar: bool) -> Elemento<'_> {
        let u = self.escala;
        let energia = peticion
            .energia()
            .expect("solo se confirma lo que tiene acción de energía");
        let destructiva = energia.es_destructiva();
        let (titulo, cuerpo, etiqueta_accion) = match peticion {
            Peticion::Apagar => (
                self.idioma.t("¿Apagar el equipo?", "Shut down the computer?"),
                self.idioma.t(
                    "El equipo se apagará por completo.",
                    "The computer will shut down completely.",
                ),
                self.idioma.t("Apagar", "Shut down"),
            ),
            Peticion::Reiniciar => (
                self.idioma.t("¿Reiniciar el equipo?", "Restart the computer?"),
                self.idioma.t(
                    "El equipo volverá a iniciarse.",
                    "The computer will restart.",
                ),
                self.idioma.t("Reiniciar", "Restart"),
            ),
            _ => (energia.titulo(), energia.mensaje(), energia.etiqueta()),
        };
        let (fondo_insignia, tinta_insignia) = if destructiva {
            let fondo = tema::rojo_suave();
            (fondo, tema::rojo_en(fondo))
        } else {
            (tema::superficie_en_tarjeta(), tema::texto())
        };
        let insignia = container(propio(energia.icono(), 20.0 * u, tinta_insignia))
            .width(40.0 * u)
            .height(40.0 * u)
            .center_x(40.0 * u)
            .center_y(40.0 * u)
            .style(move |_| container::Style {
                background: Some(fondo_insignia.into()),
                border: Border {
                    radius: (20.0 * u).into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let acento_primario = if destructiva {
            tema::rojo()
        } else {
            self.acento()
        };
        let tinta_primario = if destructiva {
            tema::tinta_sobre(tema::rojo())
        } else {
            self.sobre_acento()
        };
        let cancelar = button(
            container(
                text(self.idioma.t("Cancelar", "Cancel"))
                    .size(14.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(tema::texto()),
            )
            .center_x(Fill)
            .center_y(48.0 * u),
        )
        .width(Fill)
        .padding(0)
        .on_press(Mensaje::ConfirmarEnergia(false))
        .style(move |_, estado| button::Style {
            background: Some(
                if estado != button::Status::Active {
                    tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.08)
                } else {
                    tema::superficie_en_tarjeta()
                }
                .into(),
            ),
            border: Border {
                radius: (24.0 * u).into(),
                width: if !aceptar { (2.0 * u).round() } else { 0.0 },
                color: tema::acento(),
            },
            ..Default::default()
        });

        let confirmar = button(
            container(
                text(etiqueta_accion)
                    .size(14.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(tinta_primario),
            )
            .center_x(Fill)
            .center_y(48.0 * u),
        )
        .width(Fill)
        .padding(0)
        .on_press(Mensaje::ConfirmarEnergia(true))
        .style(move |_, estado| button::Style {
            background: Some(
                if estado != button::Status::Active {
                    tema::mezclar(acento_primario, Color::WHITE, 0.10)
                } else {
                    acento_primario
                }
                .into(),
            ),
            border: Border {
                radius: (24.0 * u).into(),
                width: if aceptar { (2.0 * u).round() } else { 0.0 },
                color: tema::tinta(),
            },
            ..Default::default()
        });

        let tarjeta = container(
            column![
                insignia,
                alto(8.0 * u),
                text(titulo)
                    .size(17.0 * u)
                    .font(peso(Weight::Bold))
                    .color(tema::texto())
                    .align_x(Center),
                alto(6.0 * u),
                text(cuerpo)
                    .size(13.0 * u)
                    .color(tema::texto2())
                    .align_x(Center),
                alto(16.0 * u),
                row![cancelar, confirmar].spacing(10.0 * u),
            ]
            .align_x(Center),
        )
        .width(Confirmacion::ANCHO * u)
        .padding(Padding {
            top: 18.0 * u,
            right: 24.0 * u,
            bottom: 20.0 * u,
            left: 24.0 * u,
        })
        .style(move |_| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: (34.0 * u).into(),
                width: u.round(),
                color: tema::borde(),
            },
            ..Default::default()
        });

        // Velo al 45 % igual que `bloqueo.rs`.
        mouse_area(
            container(mouse_area(tarjeta).on_press(Mensaje::Fotograma))
                .center_x(Fill)
                .center_y(Fill)
                .style(|_| container::Style {
                    background: Some(
                        Color {
                            a: 0.45,
                            ..Color::BLACK
                        }
                        .into(),
                    ),
                    ..Default::default()
                }),
        )
        .on_press(Mensaje::ConfirmarEnergia(false))
        .into()
    }

    // 0.1.1
    fn lista_de_usuarios(&self, entrada: f32) -> Elemento<'_> {
        let u = self.escala;
        let (filas, k, arriba) = self.lista();
        let foco = match self.foco {
            Foco::Ninguno => None,
            Foco::Puntero(y) => Some(y),
            Foco::Fila(i) => {
                Some(i as f32 * (lente::ALTO + lente::HUECO) * k + lente::ALTO * k / 2.0)
            }
        };
        let puestas = lente::disponer(filas, foco, k, self.fuerza.valor());
        let primera = puestas.first().map_or(0.0, |fila| fila.0);
        let mut lista = column![].spacing(lente::HUECO * k).align_x(Left);
        for (i, (_, escala)) in puestas.into_iter().enumerate() {
            // La fila señalada se sale hacia la izquierda y las vecinas la
            // siguen a medias: la lista se curva como el canto de una rueda
            // que gira bajo el puntero.
            let sangria = (SALIENTE * k * (1.0 - lente::cercania(escala))).round();
            lista = lista.push(row![
                Space::new().width(sangria),
                self.fila(i, escala * k, entrada)
            ]);
        }
        let mut capa = column![
            alto(arriba + primera + (1.0 - entrada) * 22.0 * u),
            // La columna no se centra por su contenido: una fila que crece
            // movería a las demás de lado. Ancho fijo y filas a la izquierda.
            // El saliente se reserva a los dos lados para que la lista en
            // reposo siga centrada.
            row![
                container(lista).width((ANCHO_LISTA + SALIENTE) * k),
                Space::new().width(SALIENTE * k),
            ],
        ]
        .width(Fill)
        .align_x(Center);
        if let Some(error) = &self.error {
            capa = capa.push(alto(10.0 * u)).push(self.pildora_aviso(
                error,
                Some(tema::rojo()),
                entrada,
            ));
        }
        capa.into()
    }

    /// Avatar y nombre sueltos sobre el fondo, como en la ventana de acceso de
    /// macOS. `s` ya trae la escala de pantalla y el aumento de la lente.
    // 0.1.1
    fn fila(&self, i: usize, s: f32, alfa: f32) -> Elemento<'_> {
        let persona = self.personas.get(i);
        // A píxeles enteros: la lente pasa por decenas de tamaños intermedios y
        // cada cuerpo de letra distinto es una tanda nueva de glifos que
        // rasterizar y guardar. Así son como mucho quince, y nítidos.
        let lado = (AVATAR_FILA * s).round();
        let cuerpo = (15.0 * s).round();
        let acento = self.acento();
        let sin_foto = match persona {
            Some(persona) => text(persona.iniciales())
                .size((lado * 0.38).round())
                .font(peso(Weight::Semibold))
                .color(blanco(alfa))
                .into(),
            None => propio("teclado", (lado * 0.5).round(), blanco(alfa)),
        };
        let con_teclado = matches!(self.foco, Foco::Fila(fila) if fila == i);
        let aro = if con_teclado {
            ((2.0 * self.escala).round(), acento)
        } else {
            (self.escala.round(), blanco(0.35))
        };
        let otro = self.idioma.t("Otro usuario", "Other user");
        let nombre = persona.map_or(otro, |p| p.nombre.as_str());
        let contenido = row![
            avatar(
                persona.and_then(|p| p.miniatura.as_ref()),
                sin_foto,
                lado,
                acento,
                aro,
                alfa
            ),
            rotulo(
                bookos_shell::recortar_texto(nombre, ANCHO_LISTA - AVATAR_FILA - 12.0, 15.0),
                cuerpo,
                peso(Weight::Semibold),
                alfa
            ),
        ]
        .spacing((12.0 * s).round())
        .align_y(Center);
        button(container(contenido).center_y(Fill))
            .padding(0)
            .height(lente::ALTO * s)
            .on_press(Mensaje::Elegir(i))
            .style(|_, _| sin_fondo())
            .into()
    }

    // 0.1.1
    fn bloque_de_acceso(&self, entrada: f32) -> Elemento<'_> {
        let (u, config) = (self.escala, self.config());
        let persona = self.persona().filter(|_| !self.manual);
        let lado = (config.avatar_tamano * u * (0.90 + 0.10 * entrada)).round();
        let sin_foto = match persona {
            Some(persona) => text(persona.iniciales())
                .size(lado * 0.34)
                .color(blanco(entrada))
                .into(),
            None => propio("teclado", lado * 0.4, blanco(entrada)),
        };
        let otro = self.idioma.t("Otro usuario", "Other user");
        let nombre = match (persona, &self.estado) {
            (Some(persona), _) => persona.nombre.as_str(),
            (None, Estado::Usuario) => otro,
            (None, _) => self.usuario.as_str(),
        };
        // Volver a la lista es pulsar en quien eres, como en macOS. Con PAM a
        // medias no se ofrece: la cancelación tendría que esperar igual.
        let volver = (!self.personas.is_empty() && !matches!(self.estado, Estado::Ocupado(_)))
            .then_some(Mensaje::Cancelar);
        let sacudida = self.sacudida();
        // El bloque mide lo mismo en cualquier pantalla y `acceso_y` es una
        // fracción: a 886 de alto el avatar se montaba sobre el reloj (visto
        // en la ventana de vista previa). 1,3 es el interlineado de iced.
        let bajo_el_reloj = self.pantalla.height * config.reloj_y
            + (config.reloj_tamano + if config.fecha { 22.0 * 1.3 } else { 0.0 } + 24.0) * u;
        let arriba = (self.pantalla.height * config.acceso_y).max(bajo_el_reloj);
        let mut col = column![
            alto(arriba + (1.0 - entrada) * 30.0 * u),
            button(avatar(
                persona.and_then(|p| p.foto.as_ref()),
                sin_foto,
                lado,
                self.acento(),
                ((2.0 * u).round(), blanco(0.35)),
                entrada
            ))
            .padding(0)
            .on_press_maybe(volver.clone())
            .style(|_, _| sin_fondo()),
            alto(12.0 * u),
            button(rotulo(
                nombre.to_string(),
                (20.0 * u).round(),
                peso(Weight::Semibold),
                entrada
            ))
            .padding(0)
            .on_press_maybe(volver.clone())
            .style(|_, _| sin_fondo()),
            alto(16.0 * u),
            container(self.campo(entrada)).padding(Padding {
                left: (sacudida * 2.0).max(0.0),
                right: (-sacudida * 2.0).max(0.0),
                ..Padding::ZERO
            }),
            alto(10.0 * u),
            self.mensaje(entrada),
        ]
        .width(Fill)
        .align_x(Center);
        if self.personas.len() > 1
            && let Some(accion_volver) = volver
        {
            col = col.push(alto(10.0 * u)).push(
                button(
                    container(
                        text(self.idioma.t("Cambiar de usuario", "Switch user"))
                            .size(12.0 * u)
                            .font(peso(Weight::Medium))
                            .color(tinta_campo(0.88 * entrada)),
                    )
                    .padding([5.0 * u, 12.0 * u]),
                )
                .padding(0)
                .on_press(accion_volver)
                .style(move |_, estado| button::Style {
                    background: Some(
                        fondo_campo(
                            if estado != button::Status::Active {
                                0.92
                            } else {
                                0.72
                            } * entrada,
                        )
                        .into(),
                    ),
                    border: Border {
                        radius: (tema::R_PILL * u).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        col.into()
    }

    // 0.1.1
    fn campo(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let acento = self.acento();
        let tinta = tinta_campo(alfa);
        let comprobando = matches!(self.estado, Estado::Ocupado(Operacion::Autenticar));
        let estilo = move |_: &iced::Theme, _| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: Border::default(),
            icon: tinta,
            placeholder: tinta_campo(0.45 * alfa),
            value: tinta,
            selection: tema::alfa(acento, 0.5),
        };
        let mut es_secreta = false;
        let (dentro, listo): (Elemento<'_>, bool) = match &self.estado {
            Estado::Usuario => (
                text_input(
                    self.idioma.t("Nombre de usuario", "Username"),
                    &self.usuario,
                )
                .id("usuario")
                .size(16.0 * u)
                .font(peso(Weight::Medium))
                .padding(0)
                .on_input(Mensaje::Usuario)
                .on_submit(Mensaje::Continuar)
                .style(estilo)
                .into(),
                !self.usuario.trim().is_empty(),
            ),
            Estado::Pregunta { tipo, .. } => {
                let secreta = matches!(tipo, TipoEntrada::Secreta);
                es_secreta = secreta;
                let con_puntos = secreta && !self.mostrar_respuesta;
                // Los «•» de `secure` salen diminutos al tamaño del texto. Se
                // deja el campo de verdad, invisible, para el teclado y el foco,
                // y encima se ven los mismos puntos del bloqueo.
                let estilo_campo = move |tema_: &iced::Theme, estado| text_input::Style {
                    value: if con_puntos { Color::TRANSPARENT } else { tinta },
                    selection: if con_puntos {
                        Color::TRANSPARENT
                    } else {
                        tema::alfa(acento, 0.5)
                    },
                    ..estilo(tema_, estado)
                };
                (
                    text_input(
                        if secreta {
                            self.idioma.t("Contraseña", "Password")
                        } else {
                            self.idioma.t("Respuesta", "Response")
                        },
                        &self.respuesta,
                    )
                    .id("respuesta")
                    .secure(secreta && !self.mostrar_respuesta)
                    .size(16.0 * u)
                    .font(peso(Weight::Medium))
                    .padding(0)
                    .on_input(Mensaje::Respuesta)
                    .on_submit(Mensaje::Continuar)
                    .style(estilo_campo)
                    .into(),
                    !self.respuesta.is_empty(),
                )
            }
            Estado::Aviso(_) | Estado::Ocupado(_) => (Space::new().width(Fill).into(), false),
        };
        let caben = ((bloqueo::CAMPO - 120.0) / (bloqueo::PUNTO + 6.0)) as usize;
        let dentro: Elemento<'_> = if (es_secreta && !self.mostrar_respuesta)
            || (comprobando && self.ultimo_escritos > 0)
        {
            let lado = bloqueo::PUNTO * u;
            let ola = (!tema::efectos_reducidos() && comprobando)
                .then(|| {
                    self.comprobando_desde
                        .map(|d| tema::fraccion(d.elapsed(), bloqueo::D_OLA))
                })
                .flatten()
                .filter(|t| *t < 1.0);
            let cuantos = if comprobando {
                self.ultimo_escritos
            } else {
                self.respuesta.chars().count()
            }
            .min(caben);
            let mut puntos = row![].spacing(6.0 * u).align_y(Center);
            for i in 0..cuantos {
                let alza = ola.map_or(0.0, |t| {
                    ((t * 2.0 - i as f32 * 0.08) * std::f32::consts::TAU)
                        .sin()
                        .max(0.0)
                        * 5.0
                        * u
                });
                let punto = container(Space::new().width(lado).height(lado)).style(move |_| {
                    container::Style {
                        background: Some(tinta.into()),
                        border: Border {
                            radius: (lado / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                });
                puntos = puntos.push(
                    container(punto)
                        .width(lado)
                        .height(lado + 10.0 * u)
                        .center_x(lado)
                        .padding(Padding {
                            bottom: alza * 2.0,
                            ..Padding::ZERO
                        })
                        .center_y(lado + 10.0 * u),
                );
            }
            // Quieto, como en el bloqueo. Con el campo vacío va solo la pista.
            if !comprobando && !self.respuesta.is_empty() {
                puntos = puntos.push(
                    container(
                        Space::new()
                            .width(2.0 * u)
                            .height(bloqueo::ALTO_CAMPO * 0.42 * u),
                    )
                    .style(move |_| container::Style {
                        background: Some(tinta_campo(0.8 * alfa).into()),
                        ..Default::default()
                    }),
                );
            }
            if comprobando {
                puntos.into()
            } else {
                // Con el campo arriba para que el clic siga cayendo en él.
                Stack::new()
                    .width(Fill)
                    .push(
                        container(puntos)
                            .height(bloqueo::ALTO_CAMPO * u)
                            .center_y(bloqueo::ALTO_CAMPO * u),
                    )
                    .push(
                        container(dentro)
                            .height(bloqueo::ALTO_CAMPO * u)
                            .center_y(bloqueo::ALTO_CAMPO * u),
                    )
                    .into()
            }
        } else {
            dentro
        };
        let borde = match self.estado {
            Estado::Aviso(_) | Estado::Ocupado(_) => tinta_campo(0.35 * alfa),
            _ if self.error.is_some() => tema::alfa(tema::rojo(), alfa),
            _ => tema::alfa(acento, 0.85 * alfa),
        };
        let (lado_boton, alto_campo) = (bloqueo::BOTON * u, bloqueo::ALTO_CAMPO * u);
        let tinta_flecha = if listo {
            tema::alfa(self.sobre_acento(), alfa)
        } else {
            tinta_campo(0.45 * alfa)
        };
        let boton = button(
            container(pictograma(
                &self.flecha,
                18.0 * u,
                18.0 * u,
                Some(tinta_flecha),
            ))
            .center_x(lado_boton)
            .center_y(lado_boton),
        )
        .padding(0)
        .on_press_maybe(listo.then_some(Mensaje::Continuar))
        .style(move |_, _| button::Style {
            background: Some(if listo {
                tema::alfa(acento, alfa).into()
            } else {
                tinta_campo(0.12 * alfa).into()
            }),
            border: Border {
                radius: (lado_boton / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let mut fila_campo = row![
            pictograma(
                &self.candado,
                16.0 * u,
                16.0 * u,
                Some(tinta_campo(0.6 * alfa))
            ),
            dentro,
        ]
        .spacing(10.0 * u)
        .align_y(Center);
        if es_secreta && !self.respuesta.is_empty() {
            let icono_ojo = if self.mostrar_respuesta {
                &self.ojo_tachado
            } else {
                &self.ojo
            };
            fila_campo = fila_campo.push(
                button(
                    container(pictograma(
                        icono_ojo,
                        16.0 * u,
                        16.0 * u,
                        Some(tinta_campo(0.65 * alfa)),
                    ))
                    .center_x(26.0 * u)
                    .center_y(26.0 * u),
                )
                .padding(0)
                .on_press(Mensaje::AlternarMostrarClave)
                .style(|_, _| sin_fondo()),
            );
        }
        fila_campo = fila_campo.push(boton);
        container(fila_campo)
            .width(bloqueo::CAMPO * u)
            .height(alto_campo)
            .padding(Padding::ZERO.left(18.0 * u).right(4.0 * u))
            .center_y(alto_campo)
            .style(move |_| container::Style {
                background: Some(fondo_campo(alfa).into()),
                border: Border {
                    radius: (alto_campo / 2.0).into(),
                    width: (2.0 * u).round(),
                    color: borde,
                },
                ..Default::default()
            })
            .into()
    }

    /// Píldora de estado bajo el campo, igual que `bloqueo::aviso`.
    fn pildora_aviso<'a>(&self, texto: &str, senal: Option<Color>, alfa: f32) -> Elemento<'a> {
        let u = self.escala;
        let alto_pildora = bloqueo::ALTO_MENSAJE * u;
        if texto.is_empty() {
            return Space::new().height(alto_pildora).into();
        }
        let mut fila = row![].spacing(7.0 * u).align_y(Center);
        if let Some(color) = senal {
            fila = fila.push(
                container(Space::new().width(7.0 * u).height(7.0 * u)).style(move |_| {
                    container::Style {
                        background: Some(tema::alfa(color, alfa).into()),
                        border: Border {
                            radius: (3.5 * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }),
            );
        }
        fila = fila.push(
            text(texto.to_string())
                .size(13.0 * u)
                .color(tinta_campo(alfa)),
        );
        container(fila)
            .height(alto_pildora)
            .center_y(alto_pildora)
            .padding([0.0, 12.0 * u])
            .style(move |_| container::Style {
                background: Some(fondo_campo(alfa).into()),
                border: Border {
                    radius: (alto_pildora / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.1
    // modificado en la 0.2.0 -- la pregunta de pam_bookos_huella invita a poner el dedo
    // modificado en la 0.2.4 -- píldora con fondo y punto indicador, igual que `bloqueo.rs`
    fn mensaje(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let (texto, senal) = match (&self.error, &self.estado) {
            (Some(error), _) => (error.as_str(), Some(tema::rojo())),
            (None, Estado::Usuario) => (
                self.idioma
                    .t("Escribe tu nombre de usuario", "Enter your username"),
                None,
            ),
            (None, Estado::Pregunta { texto, .. }) if texto == crate::PREGUNTA_HUELLA => (
                self.idioma.t(
                    "Escribe tu contraseña o pon el dedo",
                    "Enter your password or use your fingerprint",
                ),
                None,
            ),
            (
                None,
                Estado::Pregunta {
                    tipo: TipoEntrada::Secreta,
                    ..
                },
            ) => (
                self.idioma
                    .t("Introduce tu contraseña", "Enter your password"),
                None,
            ),
            (None, Estado::Pregunta { texto, .. }) => (texto.as_str(), None),
            (None, Estado::Aviso(aviso)) => (aviso.as_str(), None),
            (None, Estado::Ocupado(Operacion::Autenticar)) => {
                (self.idioma.t("Comprobando…", "Verifying…"), None)
            }
            (None, Estado::Ocupado(Operacion::Iniciar)) => (
                self.idioma.t("Entrando en BookOS…", "Signing in…"),
                None,
            ),
            (None, Estado::Ocupado(Operacion::Cancelar)) => {
                (self.idioma.t("Cancelando…", "Cancelling…"), None)
            }
        };
        let principal = self.pildora_aviso(texto, senal, alfa);
        if self.caps_lock {
            column![
                principal,
                alto(4.0 * u),
                self.pildora_aviso(
                    self.idioma
                        .t("Bloq Mayús está activado", "Caps Lock is on"),
                    Some(tema::amarillo()),
                    alfa,
                ),
            ]
            .align_x(Center)
            .into()
        } else {
            principal
        }
    }
}
