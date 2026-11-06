use bookos_shell::bloqueo::{self, Peticion};
use bookos_shell::{Icono, Link, Network, icono, tema};
use iced::font::{Family, Weight};
use iced::widget::{
    Space, Stack, button, column, container, image, mouse_area, row, svg, text, text_input,
};
use iced::{Border, Center, Color, Element, Fill, Font, Left, Padding, Right};

use crate::sistema::{DISTRIBUCIONES, Idioma};
use crate::usuarios::Persona;
use crate::{Acceso, Estado, Foco, Mensaje, Operacion, Panel, SACUDIDA, TipoEntrada, lente};

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
const ANCHO_PANEL: f32 = 296.0;

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
                background: Some(blanco(0.12 * alfa).into()),
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
        let entrada =
            tema::C_ENTRADA.eval(tema::avance(self.cambio_desde.elapsed(), tema::D_PAGINA));
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
        capas = capas.push(self.reloj()).push(if self.en_lista() {
            self.lista_de_usuarios(entrada)
        } else {
            self.bloque_de_acceso(entrada)
        });
        capas = capas.push(self.fila_de_estado());
        let abierto = self.menu.valor();
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
        container(capas)
            .style(|_| container::Style {
                background: Some(FONDO.into()),
                ..Default::default()
            })
            .into()
    }

    /// Hora y fecha, en el mismo orden y blanco que el bloqueo: al entrar y al
    /// bloquear es la misma pantalla.
    // modificado en la 0.2.0 -- antes la fecha iba encima y con el tono de la paleta.
    // 0.1.1
    fn reloj(&self) -> Elemento<'_> {
        let (u, config) = (self.escala, self.config());
        let familia = Family::Name(config.reloj_fuente.familia());
        let tinta = Color::WHITE;
        let mut reloj = column![
            alto(self.pantalla.height * config.reloj_y),
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
                    .color(tema::alfa(tinta, 0.75))
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
    fn fila_de_estado(&self) -> Elemento<'_> {
        let u = self.escala;
        let lado = ICONO_ESTADO * u;
        let etiqueta = |texto: &str| {
            rotulo(
                texto.to_string(),
                (13.0 * u).round(),
                peso(Weight::Semibold),
                1.0,
            )
        };

        let mut fila = row![].spacing(6.0 * u).align_y(Center);
        if let Some(distribucion) = &self.teclado {
            fila = fila.push(
                self.boton_estado(
                    row![
                        etiqueta(distribucion),
                        con_sombra(&self.tecla, lado, 1.0, u)
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
                    .color(*tinta)
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
            }) => propio("cable", lado, Color::WHITE),
            // Sin red son los mismos arcos apagados: un icono tachado más en
            // la fila llama más la atención que el dato que da.
            Some(Network { up: true, .. }) => con_sombra(&self.wifi, lado, 1.0, u),
            Some(Network { up: false, .. }) | None => con_sombra(&self.wifi, lado, 0.35, u),
        };
        fila = fila.push(self.boton_estado(icono_red, Panel::Red, true));
        fila = fila.push(self.boton_estado(
            con_sombra(&self.encendido, lado, 1.0, u),
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
        let (contenido, ancho) = match panel {
            Panel::Energia => (self.menu_de_energia(alfa), bloqueo::ANCHO_MENU),
            Panel::Idioma => (self.menu_de_idioma(alfa), ANCHO_PANEL),
            Panel::Bateria => (self.menu_de_bateria(alfa), ANCHO_PANEL),
            Panel::Red => (self.menu_de_red(alfa), ANCHO_PANEL),
        };
        let tarjeta = container(contenido)
            .width(ancho * u)
            .clip(true)
            .style(move |_| container::Style {
                background: Some(tema::alfa(bloqueo::FONDO_CAMPO, 0.94 * alfa).into()),
                border: Border {
                    radius: (tema::R_TARJETA * u).into(),
                    width: u.round(),
                    color: blanco(0.12 * alfa),
                },
                ..Default::default()
            });
        // El clic dentro de la tarjeta no debe cerrar el emergente.
        let protegida = mouse_area(tarjeta).on_press(Mensaje::Fotograma);
        container(protegida)
            .width(Fill)
            .align_x(Right)
            .padding(Padding {
                top: (MARGEN + BOTON_ESTADO + 6.0 - (1.0 - avance) * 18.0) * u,
                right: (MARGEN - (BOTON_ESTADO - ICONO_ESTADO) / 2.0) * u,
                ..Padding::ZERO
            })
            .into()
    }

    // 0.1.1
    fn menu_de_idioma(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let acento = self.acento();
        let sobre = self.sobre_acento();
        let tinta = blanco(alfa);
        let sub = blanco(0.55 * alfa);

        // Selector segmentado de idioma (Español / English).
        let mut fila_idiomas = row![].spacing(6.0 * u);
        for idioma in Idioma::TODOS {
            let activo = self.idioma == idioma;
            let color_txt = if activo {
                tema::alfa(sobre, alfa)
            } else {
                blanco(0.82 * alfa)
            };
            fila_idiomas = fila_idiomas.push(
                button(
                    container(
                        text(idioma.etiqueta())
                            .size(13.0 * u)
                            .font(peso(Weight::Semibold))
                            .color(color_txt),
                    )
                    .center_x(Fill)
                    .center_y(30.0 * u),
                )
                .width(Fill)
                .padding(0)
                .on_press(Mensaje::ElegirIdioma(idioma))
                .style(move |_, estado| button::Style {
                    background: Some(if activo {
                        tema::alfa(acento, alfa).into()
                    } else if estado != button::Status::Active {
                        blanco(0.14 * alfa).into()
                    } else {
                        blanco(0.07 * alfa).into()
                    }),
                    border: Border {
                        radius: (8.0 * u).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }

        let cabecera = column![
            text(self.idioma.t("Idioma", "Language"))
                .size(12.0 * u)
                .font(peso(Weight::Semibold))
                .color(sub),
            alto(6.0 * u),
            fila_idiomas,
            alto(12.0 * u),
            row![
                text(self.idioma.t("Teclado", "Keyboard"))
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(sub),
                Space::new().width(Fill),
                text("Alt + Shift").size(11.0 * u).color(blanco(0.4 * alfa)),
            ]
            .align_y(Center),
        ]
        .padding([14.0 * u, 16.0 * u]);

        let mut lista = column![];
        for (codigo, nombre_es, nombre_en) in DISTRIBUCIONES {
            let activo = self.teclado_codigo == *codigo;
            let nombre = self.idioma.t(nombre_es, nombre_en);
            let marca: Elemento<'_> = if activo {
                pictograma(
                    &self.comprobado,
                    14.0 * u,
                    14.0 * u,
                    Some(tema::alfa(acento, alfa)),
                )
            } else {
                Space::new().width(14.0 * u).height(14.0 * u).into()
            };
            let fila = row![
                marca,
                text(nombre)
                    .size(14.0 * u)
                    .font(peso(if activo { Weight::Bold } else { Weight::Medium }))
                    .color(tinta),
                Space::new().width(Fill),
                text(codigo.to_uppercase())
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(sub),
            ]
            .spacing(10.0 * u)
            .align_y(Center);
            lista = lista.push(
                button(container(fila).center_y(Fill))
                    .padding([0.0, 16.0 * u])
                    .width(Fill)
                    .height(34.0 * u)
                    .on_press(Mensaje::ElegirTeclado((*codigo).to_string()))
                    .style(move |_, estado| button::Style {
                        background: (estado != button::Status::Active)
                            .then(|| blanco(0.10 * alfa).into()),
                        ..Default::default()
                    }),
            );
        }

        column![
            cabecera,
            separador_panel(ANCHO_PANEL - 32.0, u, alfa),
            alto(4.0 * u),
            lista,
            alto(8.0 * u),
        ]
        .into()
    }

    // 0.1.1
    // modificado en la 0.2.0 -- glifos blancos, como el bloqueo: las insignias de color son de aplicación
    fn menu_de_bateria(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let acento = self.acento();
        let tinta = blanco(alfa);
        let sub = blanco(0.58 * alfa);
        let detalle = &self.bateria_detalle;

        let mut col = column![];
        if let Some(bat) = &detalle.dato {
            let pct = bat.percent.min(100);
            let estado_txt = if bat.charging {
                match bat.minutes {
                    Some(m) if m > 0 => format!(
                        "{} · {} h {:02} min",
                        self.idioma.t("Cargando", "Charging"),
                        m / 60,
                        m % 60
                    ),
                    _ => self.idioma.t("Cargando", "Charging").to_string(),
                }
            } else if bat.plugged {
                if let Some(tope) = detalle.tope {
                    format!(
                        "{} ({tope} %)",
                        self.idioma
                            .t("Enchufado · Tope de carga", "Plugged in · Charge limit")
                    )
                } else {
                    self.idioma
                        .t("Conectado a la corriente", "Plugged in")
                        .to_string()
                }
            } else {
                match bat.minutes {
                    Some(m) if m > 0 => format!(
                        "{} h {:02} min {}",
                        m / 60,
                        m % 60,
                        self.idioma.t("restantes", "remaining")
                    ),
                    _ => self.idioma.t("Con batería", "On battery").to_string(),
                }
            };

            let ancho_barra = (ANCHO_PANEL - 32.0) * u;
            let ancho_lleno = (ancho_barra * (pct as f32 / 100.0)).max(6.0 * u);
            let color_barra = if bat.charging {
                tema::verde()
            } else if pct <= 15 {
                tema::rojo()
            } else {
                acento
            };

            let barra = Stack::new()
                .push(
                    container(Space::new().width(ancho_barra).height(8.0 * u)).style(move |_| {
                        container::Style {
                            background: Some(blanco(0.14 * alfa).into()),
                            border: Border {
                                radius: (4.0 * u).into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }),
                )
                .push(
                    container(Space::new().width(ancho_lleno).height(8.0 * u)).style(move |_| {
                        container::Style {
                            background: Some(tema::alfa(color_barra, alfa).into()),
                            border: Border {
                                radius: (4.0 * u).into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }),
                );

            col = col.push(
                column![
                    row![
                        text(self.idioma.t("Batería", "Battery"))
                            .size(14.0 * u)
                            .font(peso(Weight::Bold))
                            .color(tinta),
                        Space::new().width(Fill),
                        text(format!("{pct} %"))
                            .size(15.0 * u)
                            .font(peso(Weight::Bold))
                            .color(tinta),
                    ]
                    .align_y(Center),
                    alto(8.0 * u),
                    barra,
                    alto(6.0 * u),
                    text(estado_txt).size(12.0 * u).color(sub),
                ]
                .padding([14.0 * u, 16.0 * u]),
            );
            col = col.push(separador_panel(ANCHO_PANEL - 32.0, u, alfa));
        }

        col = col.push(
            container(
                text(self.idioma.t("Modo de energía", "Power mode"))
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(sub),
            )
            .padding(Padding {
                top: 10.0 * u,
                left: 16.0 * u,
                right: 16.0 * u,
                bottom: 4.0 * u,
            }),
        );

        let perfiles = [
            (
                "power-saver",
                "perfil-ahorro",
                self.idioma.t("Ahorro de energía", "Power Saver"),
            ),
            (
                "balanced",
                "perfil-equilibrado",
                self.idioma.t("Equilibrado", "Balanced"),
            ),
            (
                "performance",
                "perfil-rendimiento",
                self.idioma.t("Alto rendimiento", "Performance"),
            ),
        ];
        for (clave, icono_nombre, nombre) in perfiles {
            if !detalle.perfiles_disponibles.is_empty()
                && !detalle.perfiles_disponibles.iter().any(|p| p == clave)
            {
                continue;
            }
            let activo = detalle.perfil == clave;
            let marca: Elemento<'_> = if activo {
                pictograma(
                    &self.comprobado,
                    14.0 * u,
                    14.0 * u,
                    Some(tema::alfa(acento, alfa)),
                )
            } else {
                Space::new().width(14.0 * u).height(14.0 * u).into()
            };
            let fila = row![
                // El activo ya lo dice la marca de la derecha: el glifo no
                // cambia de color.
                propio(icono_nombre, 18.0 * u, blanco(alfa)),
                text(nombre)
                    .size(14.0 * u)
                    .font(peso(if activo { Weight::Bold } else { Weight::Medium }))
                    .color(tinta),
                Space::new().width(Fill),
                marca,
            ]
            .spacing(10.0 * u)
            .align_y(Center);
            col = col.push(
                button(container(fila).center_y(Fill))
                    .padding([0.0, 16.0 * u])
                    .width(Fill)
                    .height(38.0 * u)
                    .on_press(Mensaje::PerfilEnergia(clave.to_string()))
                    .style(move |_, estado| button::Style {
                        background: (estado != button::Status::Active)
                            .then(|| blanco(0.10 * alfa).into()),
                        ..Default::default()
                    }),
            );
        }
        col.push(alto(8.0 * u)).into()
    }

    // 0.1.1
    fn menu_de_red(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let acento = self.acento();
        let sobre = self.sobre_acento();
        let tinta = blanco(alfa);
        let sub = blanco(0.58 * alfa);
        let wifi = &self.wifi_estado;

        let texto_boton = if wifi.encendida {
            self.idioma.t("Activada", "On")
        } else {
            self.idioma.t("Desactivada", "Off")
        };
        let encendida = wifi.encendida;
        let interruptor = button(
            container(
                text(texto_boton)
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(if encendida {
                        tema::alfa(sobre, alfa)
                    } else {
                        blanco(0.75 * alfa)
                    }),
            )
            .padding([4.0 * u, 10.0 * u]),
        )
        .padding(0)
        .on_press(Mensaje::WifiAlternar(!encendida))
        .style(move |_, _| button::Style {
            background: Some(if encendida {
                tema::alfa(acento, alfa).into()
            } else {
                blanco(0.15 * alfa).into()
            }),
            border: Border {
                radius: (12.0 * u).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut cabecera = row![
            con_sombra(&self.wifi, 18.0 * u, alfa, u),
            text("Wi-Fi")
                .size(15.0 * u)
                .font(peso(Weight::Bold))
                .color(tinta),
            Space::new().width(Fill),
        ]
        .spacing(8.0 * u)
        .align_y(Center);

        if wifi.encendida {
            let recargar = button(
                container(propio("reiniciar", 15.0 * u, blanco(0.75 * alfa)))
                    .center_x(26.0 * u)
                    .center_y(26.0 * u),
            )
            .padding(0)
            .on_press(Mensaje::WifiRefrescar)
            .style(move |_, estado| button::Style {
                background: (estado != button::Status::Active).then(|| blanco(0.14 * alfa).into()),
                border: Border {
                    radius: (13.0 * u).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            cabecera = cabecera.push(recargar);
        }
        cabecera = cabecera.push(interruptor);

        let mut col = column![container(cabecera).padding([14.0 * u, 16.0 * u])];
        if wifi.cable {
            col = col.push(
                container(
                    row![
                        propio("cable", 16.0 * u, tema::alfa(tema::verde(), alfa)),
                        text(
                            self.idioma
                                .t("Conectado por cable Ethernet", "Ethernet cable connected")
                        )
                        .size(12.0 * u)
                        .color(sub),
                    ]
                    .spacing(8.0 * u)
                    .align_y(Center),
                )
                .padding([0.0, 16.0 * u]),
            );
            col = col.push(alto(8.0 * u));
        }

        col = col.push(separador_panel(ANCHO_PANEL - 32.0, u, alfa));

        if let Some(err) = self.wifi_error.as_deref().or(wifi.error.as_deref()) {
            col = col.push(
                container(
                    text(err)
                        .size(12.0 * u)
                        .color(tema::alfa(tema::rojo(), alfa)),
                )
                .padding([8.0 * u, 16.0 * u]),
            );
        }

        if self.wifi_cargando {
            col = col.push(
                container(
                    text(self.idioma.t("Buscando redes…", "Scanning networks…"))
                        .size(13.0 * u)
                        .color(sub),
                )
                .padding([16.0 * u, 16.0 * u]),
            );
            return col.into();
        }

        if !wifi.encendida {
            col = col.push(
                container(
                    text(self.idioma.t(
                        "Activa el Wi-Fi para ver las redes disponibles.",
                        "Turn on Wi-Fi to see available networks.",
                    ))
                    .size(13.0 * u)
                    .color(sub),
                )
                .padding([16.0 * u, 16.0 * u]),
            );
            return col.into();
        }

        if let Some(ssid) = &self.wifi_conectar_a {
            let estilo_input = move |_: &iced::Theme, _| text_input::Style {
                background: blanco(0.10 * alfa).into(),
                border: Border {
                    radius: (8.0 * u).into(),
                    width: u.round(),
                    color: tema::alfa(acento, 0.85 * alfa),
                },
                icon: tinta,
                placeholder: blanco(0.45 * alfa),
                value: tinta,
                selection: tema::alfa(acento, 0.5),
            };
            let listo = !self.wifi_clave.is_empty();
            let form = column![
                text(format!(
                    "{} «{ssid}»",
                    self.idioma.t("Contraseña para", "Password for")
                ))
                .size(13.0 * u)
                .font(peso(Weight::Semibold))
                .color(tinta),
                alto(8.0 * u),
                text_input(
                    self.idioma.t("Contraseña Wi-Fi", "Wi-Fi password"),
                    &self.wifi_clave
                )
                .id("wifi_clave")
                .secure(true)
                .size(14.0 * u)
                .padding([8.0 * u, 10.0 * u])
                .on_input(Mensaje::WifiClave)
                .on_submit(Mensaje::WifiConectar)
                .style(estilo_input),
                alto(10.0 * u),
                row![
                    button(
                        container(
                            text(self.idioma.t("Cancelar", "Cancel"))
                                .size(13.0 * u)
                                .font(peso(Weight::Semibold))
                                .color(tinta)
                        )
                        .center_x(Fill)
                        .center_y(30.0 * u)
                    )
                    .width(Fill)
                    .padding(0)
                    .on_press(Mensaje::WifiCancelarClave)
                    .style(move |_, _| button::Style {
                        background: Some(blanco(0.12 * alfa).into()),
                        border: Border {
                            radius: (8.0 * u).into(),
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
                                    blanco(0.4 * alfa)
                                })
                        )
                        .center_x(Fill)
                        .center_y(30.0 * u)
                    )
                    .width(Fill)
                    .padding(0)
                    .on_press_maybe(listo.then_some(Mensaje::WifiConectar))
                    .style(move |_, _| button::Style {
                        background: Some(if listo {
                            tema::alfa(acento, alfa).into()
                        } else {
                            blanco(0.08 * alfa).into()
                        }),
                        border: Border {
                            radius: (8.0 * u).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ]
                .spacing(8.0 * u),
            ]
            .padding([14.0 * u, 16.0 * u]);
            return col.push(form).into();
        }

        if wifi.redes.is_empty() {
            col = col.push(
                container(
                    text(
                        self.idioma
                            .t("No se encontraron redes Wi-Fi", "No Wi-Fi networks found"),
                    )
                    .size(13.0 * u)
                    .color(sub),
                )
                .padding([16.0 * u, 16.0 * u]),
            );
            return col.into();
        }

        col = col.push(alto(4.0 * u));
        for (i, red) in wifi.redes.iter().enumerate() {
            let marca: Elemento<'_> = if red.activa {
                pictograma(
                    &self.comprobado,
                    14.0 * u,
                    14.0 * u,
                    Some(tema::alfa(acento, alfa)),
                )
            } else {
                Space::new().width(14.0 * u).height(14.0 * u).into()
            };
            let candado_red: Elemento<'_> = if red.segura {
                pictograma(&self.candado, 13.0 * u, 13.0 * u, Some(sub))
            } else {
                Space::new().width(0.0).into()
            };
            let fila = row![
                marca,
                text(bookos_shell::recortar_texto(&red.ssid, 165.0, 14.0))
                    .size(14.0 * u)
                    .font(peso(if red.activa {
                        Weight::Bold
                    } else {
                        Weight::Medium
                    }))
                    .color(tinta),
                Space::new().width(Fill),
                candado_red,
                text(format!("{} %", red.senal)).size(12.0 * u).color(sub),
            ]
            .spacing(8.0 * u)
            .align_y(Center);

            col = col.push(
                button(container(fila).center_y(Fill))
                    .padding([0.0, 16.0 * u])
                    .width(Fill)
                    .height(36.0 * u)
                    .on_press(Mensaje::WifiElegir(i))
                    .style(move |_, estado| button::Style {
                        background: (estado != button::Status::Active)
                            .then(|| blanco(0.10 * alfa).into()),
                        ..Default::default()
                    }),
            );
        }

        if wifi.ssid_actual.is_some() {
            col = col
                .push(alto(4.0 * u))
                .push(separador_panel(ANCHO_PANEL - 32.0, u, alfa))
                .push(
                    button(
                        container(
                            text(self.idioma.t("Desconectar", "Disconnect"))
                                .size(13.0 * u)
                                .font(peso(Weight::Semibold))
                                .color(tema::alfa(tema::rojo(), alfa)),
                        )
                        .center_x(Fill)
                        .center_y(34.0 * u),
                    )
                    .width(Fill)
                    .padding(0)
                    .on_press(Mensaje::WifiDesconectar)
                    .style(move |_, estado| button::Style {
                        background: (estado != button::Status::Active)
                            .then(|| blanco(0.08 * alfa).into()),
                        ..Default::default()
                    }),
                );
        } else {
            col = col.push(alto(8.0 * u));
        }
        col.into()
    }

    // 0.1.1
    // modificado en la 0.2.0 -- glifos blancos, como el bloqueo: las insignias de color son de aplicación
    fn menu_de_energia(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let tinta = blanco(alfa);
        let entradas: Vec<_> = bloqueo::entradas()
            .iter()
            .filter(|(_, _, peticion)| *peticion != Peticion::CerrarSesion)
            .collect();
        let mut filas = column![];
        for (i, (etiqueta, icono, peticion)) in entradas.iter().enumerate() {
            let etiqueta_traducida = match peticion {
                Peticion::Apagar => self.idioma.t("Apagar", "Shut Down"),
                Peticion::Reiniciar => self.idioma.t("Reiniciar", "Restart"),
                Peticion::Suspender => self.idioma.t("Suspender", "Sleep"),
                Peticion::Hibernar => self.idioma.t("Hibernar", "Hibernate"),
                Peticion::CerrarSesion => *etiqueta,
            };
            let fila = row![
                propio(icono, 18.0 * u, blanco(alfa)),
                text(etiqueta_traducida)
                    .size(15.0 * u)
                    .font(peso(Weight::Bold))
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
                            .then(|| blanco(0.10 * alfa).into()),
                        ..Default::default()
                    }),
            );
            if i + 1 < entradas.len() {
                filas = filas.push(separador_panel(bloqueo::ANCHO_MENU - 36.0, u, alfa));
            }
        }
        filas.into()
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
            capa = capa.push(alto(10.0 * u)).push(
                text(error)
                    .size(tema::T_CUERPO * u)
                    .color(tema::alfa(tema::rojo(), entrada)),
            );
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
        column![
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
            .on_press_maybe(volver)
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
        .align_x(Center)
        .into()
    }

    // 0.1.1
    fn campo(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let acento = self.acento();
        let estilo = move |_: &iced::Theme, _| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: Border::default(),
            icon: blanco(alfa),
            placeholder: blanco(0.45 * alfa),
            value: blanco(alfa),
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
                    value: if con_puntos {
                        Color::TRANSPARENT
                    } else {
                        blanco(alfa)
                    },
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
        let dentro: Elemento<'_> = if es_secreta && !self.mostrar_respuesta {
            let lado = bloqueo::PUNTO * u;
            let mut puntos = row![].spacing(6.0 * u).align_y(Center);
            // Con el ojo y el botón al lado caben menos que en el bloqueo.
            let caben = ((bloqueo::CAMPO - 120.0) / (bloqueo::PUNTO + 6.0)) as usize;
            for _ in 0..self.respuesta.chars().count().min(caben) {
                puntos = puntos.push(container(Space::new().width(lado).height(lado)).style(
                    move |_| container::Style {
                        background: Some(blanco(alfa).into()),
                        border: Border {
                            radius: (lado / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ));
            }
            // Quieto, como en el bloqueo. Con el campo vacío va solo la pista.
            if !self.respuesta.is_empty() {
                puntos = puntos.push(
                    container(
                        Space::new()
                            .width(2.0 * u)
                            .height(bloqueo::ALTO_CAMPO * 0.42 * u),
                    )
                    .style(move |_| container::Style {
                        background: Some(blanco(0.8 * alfa).into()),
                        ..Default::default()
                    }),
                );
            }
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
        } else {
            dentro
        };
        let borde = match self.estado {
            Estado::Aviso(_) | Estado::Ocupado(_) => blanco(0.35 * alfa),
            _ if self.error.is_some() => tema::alfa(tema::rojo(), alfa),
            _ => tema::alfa(acento, 0.85 * alfa),
        };
        let (lado_boton, alto_campo) = (bloqueo::BOTON * u, bloqueo::ALTO_CAMPO * u);
        let tinta_flecha = if listo {
            tema::alfa(self.sobre_acento(), alfa)
        } else {
            blanco(0.45 * alfa)
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
                blanco(0.12 * alfa).into()
            }),
            border: Border {
                radius: (lado_boton / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let mut fila_campo = row![
            pictograma(&self.candado, 16.0 * u, 16.0 * u, Some(blanco(0.6 * alfa))),
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
                        Some(blanco(0.65 * alfa)),
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
                background: Some(tema::alfa(bloqueo::FONDO_CAMPO, 0.82 * alfa).into()),
                border: Border {
                    radius: (alto_campo / 2.0).into(),
                    width: (2.0 * u).round(),
                    color: borde,
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.1
    // modificado en la 0.2.0 -- la pregunta de pam_bookos_huella invita a poner el dedo
    fn mensaje(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let (texto, color) = match (&self.error, &self.estado) {
            (Some(error), _) => (error.as_str(), tema::rojo()),
            (None, Estado::Usuario) => (
                self.idioma
                    .t("Escribe tu nombre de usuario", "Enter your username"),
                blanco(0.55),
            ),
            // pam_unix pregunta a secas «Password:» o «Contraseña:»; el
            // campo ya lo dice, y en crudo parecía texto de terminal.
            (None, Estado::Pregunta { texto, .. }) if texto == crate::PREGUNTA_HUELLA => (
                self.idioma.t(
                    "Escribe tu contraseña o pon el dedo",
                    "Enter your password or use your fingerprint",
                ),
                blanco(0.55),
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
                blanco(0.55),
            ),
            (None, Estado::Pregunta { texto, .. }) => (texto.as_str(), blanco(0.55)),
            (None, Estado::Aviso(aviso)) => (aviso.as_str(), blanco(0.8)),
            (None, Estado::Ocupado(Operacion::Autenticar)) => {
                (self.idioma.t("Comprobando…", "Verifying…"), blanco(0.55))
            }
            (None, Estado::Ocupado(Operacion::Iniciar)) => (
                self.idioma.t("Entrando en BookOS…", "Signing in…"),
                blanco(0.55),
            ),
            (None, Estado::Ocupado(Operacion::Cancelar)) => {
                (self.idioma.t("Cancelando…", "Cancelling…"), blanco(0.55))
            }
        };
        let principal = text(texto)
            .size(tema::T_CUERPO * u)
            .color(tema::alfa(color, color.a * alfa))
            .width(420.0 * u)
            .align_x(Center);
        if self.caps_lock {
            column![
                principal,
                alto(4.0 * u),
                text(self.idioma.t("⇪ Bloq Mayús activado", "⇪ Caps Lock is on"))
                    .size(12.0 * u)
                    .font(peso(Weight::Semibold))
                    .color(tema::alfa(tema::amarillo(), alfa))
                    .width(420.0 * u)
                    .align_x(Center),
            ]
            .align_x(Center)
            .into()
        } else {
            principal.into()
        }
    }
}
