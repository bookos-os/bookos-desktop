//! «Acerca de este PC»: la tarjeta con lo que es esta máquina.
//!
//! La abre el menú del logo. Antes lanzaba `kinfocenter`, que es el panel de
//! otro escritorio y enseña la versión de Plasma y de Qt: cosas del sistema
//! anfitrión, no de BookOS.
//!
//! **Todo sale de sysfs y de `/proc`**, sin procesos hijos ni D-Bus, igual que
//! el panel: el equipo de DMI, la CPU de `/proc/cpuinfo`, la memoria de
//! `/proc/meminfo` y la base de `/etc/os-release`. El único fichero grande es
//! `pci.ids`, que se lee una vez al abrir la ventana y solo para traducir el
//! identificador de la tarjeta gráfica a su nombre comercial —el kernel no lo
//! sabe, solo tiene `0x64a0`.
//!
//! El sistema que se enseña bajo el título es **BookOS**, con la versión del
//! propio compositor. `/etc/os-release` describe Fedora, que es lo que hay
//! debajo, no lo que el usuario está usando; eso sale en su propio renglón,
//! «Base», más abajo.
//!
//! El número de serie sale **tapado** y se destapa con el ojo: es un dato que
//! se pide por teléfono al servicio técnico y no hace falta que quede a la
//! vista en cada captura de pantalla. `/sys/class/dmi/id/product_serial` es
//! 0400 de root; la regla `71-bookos-serie.rules` lo deja legible al grupo
//! `wheel`. Sin ella el ojo destapa un aviso en vez del número.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length, Padding, Point, Rectangle};
use iced_widget::{Space, column, container, row, stack, svg, text};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, Tecla, t};

// Medidas de la mesa «Acerca de este equipo» del lienzo, aprobada 1:1:
// diálogo del HIG (420, radio 26, relleno 12 24 24) con la ficha y las
// acciones en tarjetas agrupadas como una página de Ajustes.
const ANCHO: f32 = 420.0;
const LADO: f32 = 24.0;
const ARRIBA: f32 = 12.0;
const ABAJO: f32 = 24.0;
/// El botón de cerrar: círculo neutro de 30, como el resto de diálogos.
const CIERRE: f32 = 30.0;
const RETRATO: f32 = 140.0;
/// Proporción de `portatil.svg` (380×217): los SVG ocupan esta caja.
const RETRATO_ANCHO: f32 = RETRATO * 380.0 / 217.0;
const NOMBRE: f32 = 28.0;
const VERSION: f32 = 18.0;
const FILA: f32 = 44.0;
const GRUPO_RADIO: f32 = 14.0;
/// Entre grupos y entre la versión y la ficha.
const HUECO: f32 = 16.0;
/// Lo que se enseña mientras el número está tapado. Siempre la misma
/// cantidad de puntos: si dependiera del número, ya diría cuánto mide.
const TAPADO: &str = "•••••••••••";
/// Destapar: cada carácter sube 4 px y aparece, 20 ms detrás del anterior,
/// en 180 ms. Tapar: los puntos bajan a su sitio de derecha a izquierda,
/// 12 ms y 150 ms, más corto porque esconder no tiene que llamar la atención.
const LETRA_SUBE: Duration = Duration::from_millis(180);
const LETRA_SUBE_PASO: Duration = Duration::from_millis(20);
const LETRA_BAJA: Duration = Duration::from_millis(150);
const LETRA_BAJA_PASO: Duration = Duration::from_millis(12);
/// Lo que dura «Ficha copiada» en la fila antes de volver a «Copiar la ficha».
const COPIADO: Duration = Duration::from_millis(1800);

/// Un renglón de la ficha.
struct Dato {
    etiqueta: &'static str,
    valor: String,
}

/// Lo que se puede pulsar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zona {
    Cerrar,
    Ojo,
    Mas,
    Copiar,
}

pub struct Acerca {
    /// Cómo se llama el equipo: «Galaxy Book5 Pro».
    modelo: String,
    /// «BookUI 0.2.0», el escritorio con su versión.
    sistema: String,
    /// «Fedora Linux 44», lo que hay debajo, en gris tras el sistema.
    base: String,
    datos: Vec<Dato>,
    /// `None` si no se pudo leer: falta la regla de udev o no se es de `wheel`.
    serie: Option<String>,
    revelado: bool,
    /// Cuándo se destapó o tapó por última vez, para la animación de las
    /// letras. `None` al abrir: los puntos salen quietos.
    serie_desde: Option<Instant>,
    copiado_desde: Option<Instant>,
    /// Se crea una vez: `Handle::from_memory` estrena identificador en cada
    /// llamada y resvg volvería a rasterizar el dibujo en cada fotograma.
    portatil: Dibujo,
    señalada: Option<Zona>,
}

impl Acerca {
    // 0.1.0
    // modificado en la 0.2.0 -- el portátil según el modelo
    // modified 0.2.2 -- la mesa del lienzo: ficha de Ajustes, disco y pantalla
    pub fn new() -> Self {
        let modelo = dmi("product_family")
            .or_else(|| dmi("product_name"))
            .unwrap_or_else(|| t("Este equipo", "This computer").into());
        // El subtítulo dice **BookUI**, no la distribución de base: es el
        // escritorio que se tiene delante, con la versión del propio
        // compositor (BookOS es el sistema entero). La base va detrás en
        // gris, corta: «Fedora Linux 44», no el PRETTY_NAME con la edición
        // de KDE, que no es lo que se está usando.
        let sistema = format!("BookUI {}", env!("CARGO_PKG_VERSION"));
        let base = match (os_release("NAME"), os_release("VERSION_ID")) {
            (Some(n), Some(v)) => format!("{n} {v}"),
            (Some(n), None) => n,
            _ => "Linux".into(),
        };
        let datos = vec![
            Dato {
                etiqueta: "Chip",
                valor: cpu().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: t("Gráficos", "Graphics"),
                valor: gpu().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: t("Memoria", "Memory"),
                valor: memoria().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: t("Almacenamiento", "Storage"),
                valor: almacenamiento().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: t("Pantalla", "Display"),
                valor: pantalla().unwrap_or_else(|| "—".into()),
            },
        ];
        let portatil = dibujo_del_modelo(&modelo);
        Self {
            modelo,
            sistema,
            base,
            datos,
            serie: dmi("product_serial"),
            revelado: false,
            serie_desde: None,
            copiado_desde: None,
            portatil,
            señalada: None,
        }
    }

    /// Dónde empieza cada cosa, de arriba abajo.
    // añadido en la 0.2.2
    fn y_ficha(&self) -> f32 {
        // El retrato sube 6 bajo la fila del cierre, como en el diseño.
        ARRIBA + CIERRE - 6.0 + RETRATO + 14.0 + NOMBRE + 2.0 + VERSION + HUECO
    }

    // añadido en la 0.2.2
    fn filas_ficha(&self) -> usize {
        self.datos.len() + 1
    }

    // añadido en la 0.2.2
    fn y_acciones(&self) -> f32 {
        self.y_ficha() + self.filas_ficha() as f32 * FILA + HUECO
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (ANCHO, self.y_acciones() + 2.0 * FILA + ABAJO)
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    // añadido en la 0.2.2
    fn rect(&self, zona: Zona) -> Rectangle {
        let ancho_grupo = ANCHO - 2.0 * LADO;
        match zona {
            // Con holgura: 30 px en una esquina se fallan.
            Zona::Cerrar => Rectangle {
                x: ANCHO - LADO - CIERRE,
                y: ARRIBA,
                width: CIERRE,
                height: CIERRE,
            }
            .expand(6.0),
            // Toda la fila del número, no solo el ojo: pulsar sobre los
            // puntos para destaparlos es lo que se intenta primero.
            Zona::Ojo => Rectangle {
                x: LADO,
                y: self.y_ficha() + self.datos.len() as f32 * FILA,
                width: ancho_grupo,
                height: FILA,
            },
            Zona::Mas => Rectangle {
                x: LADO,
                y: self.y_acciones(),
                width: ancho_grupo,
                height: FILA,
            },
            Zona::Copiar => Rectangle {
                x: LADO,
                y: self.y_acciones() + FILA,
                width: ancho_grupo,
                height: FILA,
            },
        }
    }

    // añadido en la 0.2.2
    fn zona_en(&self, x: f32, y: f32) -> Option<Zona> {
        let p = Point::new(x, y);
        [Zona::Cerrar, Zona::Ojo, Zona::Mas, Zona::Copiar]
            .into_iter()
            .find(|z| self.rect(*z).contains(p))
    }

    // 0.1.0
    // modified 0.2.2 -- realce de cada fila pulsable
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let zona = punto.and_then(|(x, y)| self.zona_en(x, y));
        let cambia = zona != self.señalada;
        self.señalada = zona;
        cambia
    }

    /// ¿Se mueve algo dentro de la tarjeta? Las letras del número de serie y
    /// «Ficha copiada» mientras dura.
    // 0.1.0
    // modified 0.2.2 -- letras del número de serie y aviso de copiado
    pub fn animando(&self) -> bool {
        let letras = self.serie_desde.is_some_and(|d| {
            d.elapsed() < LETRA_SUBE + LETRA_SUBE_PASO * TAPADO.chars().count().max(20) as u32
        });
        letras || self.copiado_desde.is_some_and(|d| d.elapsed() < COPIADO)
    }

    // 0.1.0
    // modified 0.2.2 -- copiar la ficha y animar el número
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        match self.zona_en(x, y)? {
            Zona::Cerrar => Some(Accion::CerrarEmergente),
            Zona::Ojo => {
                self.revelado = !self.revelado;
                self.serie_desde = (!tema::efectos_reducidos()).then(Instant::now);
                None
            }
            // «Más información» abre «Acerca del portátil» en los ajustes.
            // Antes se pasaba `--acerca`, que bookos-settings no conoce:
            // arrancaba sin error en su portada.
            Zona::Mas => Some(Accion::Lanzar("bookos-settings --page acerca".into())),
            Zona::Copiar => {
                self.copiado_desde = Some(Instant::now());
                Some(Accion::CopiarTexto(self.ficha_en_texto()))
            }
        }
    }

    /// La ficha en texto plano para el portapapeles. El número de serie solo
    /// si está a la vista: copiarlo tapado lo sacaría sin que se haya visto.
    // añadido en la 0.2.2
    fn ficha_en_texto(&self) -> String {
        let mut texto = format!("{}\n{} · {}\n", self.modelo, self.sistema, self.base);
        for d in &self.datos {
            let _ = writeln!(texto, "{}: {}", d.etiqueta, d.valor);
        }
        if self.revelado
            && let Some(serie) = &self.serie
        {
            let _ = writeln!(texto, "{}: {serie}", t("Número de serie", "Serial number"));
        }
        texto
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// Los colores, como una página de Ajustes: en claro el diálogo es el
    /// lienzo gris y las filas blancas; en oscuro el diálogo es la tarjeta y
    /// las filas un escalón más claras. Todos opacos.
    // añadido en la 0.2.2
    fn paleta(&self) -> Paleta {
        let claro = tema::es_claro();
        let (fondo, fila) = if claro {
            (tema::bg(), tema::card())
        } else {
            (tema::card(), tema::superficie_en_tarjeta())
        };
        let tx = tema::texto();
        Paleta {
            fondo,
            fila,
            tx,
            // #6C6C70 en claro (HIG §2: el #8E8E93 no llega a 4,5:1 sobre blanco).
            tx2: if claro {
                Color::from_rgb8(0x6c, 0x6c, 0x70)
            } else {
                tema::texto2()
            },
            div: tema::mezclar(fila, tx, 0.08),
            realce: tema::mezclar(fila, tx, 0.06),
        }
    }

    /// El portátil de arriba.
    // 0.1.0
    fn retrato(&self) -> PanelElement<'_> {
        let dibujo: PanelElement<'_> = match &self.portatil {
            Dibujo::Vector(h) => svg(h.clone())
                .width(Length::Fixed(RETRATO_ANCHO))
                .height(Length::Fixed(RETRATO))
                .into(),
            // El render ocupa el alto del retrato y su ancho propio (1,54:1).
            Dibujo::Render(h) => iced_widget::image(h.clone())
                .height(Length::Fixed(RETRATO))
                .into(),
        };
        container(dibujo)
            .width(Length::Fill)
            .align_x(Horizontal::Center)
            .into()
    }

    /// El número de serie, letra a letra, para animarlas por separado.
    // añadido en la 0.2.2
    fn serie_animada(&self, p: &Paleta) -> PanelElement<'static> {
        let texto = match (self.revelado, &self.serie) {
            (false, _) => TAPADO,
            (true, Some(serie)) => serie.as_str(),
            (true, None) => t("Sin permiso para leerlo", "No permission to read it"),
        };
        let n = texto.chars().count();
        let pasado = self.serie_desde.map(|d| d.elapsed());
        let mut fila = row![].align_y(Vertical::Center);
        for (i, c) in texto.chars().enumerate() {
            // Destapar: de izquierda a derecha, subiendo. Tapar: de derecha a
            // izquierda, bajando. Sin animación (al abrir o con efectos
            // reducidos), quietas.
            let (avance, desde) = match pasado {
                Some(e) if self.revelado => {
                    (fraccion(e, LETRA_SUBE_PASO * i as u32, LETRA_SUBE), 4.0)
                }
                Some(e) => (
                    fraccion(e, LETRA_BAJA_PASO * (n - 1 - i) as u32, LETRA_BAJA),
                    -4.0,
                ),
                None => (1.0, 0.0),
            };
            let curva = 1.0 - (1.0 - avance).powi(3);
            let dy = desde * (1.0 - curva);
            // Aparece mezclando su color con el de la fila, no con alfa: igual
            // en pantalla y sin rellenos translúcidos.
            let color = tema::mezclar(p.fila, p.tx2, curva);
            fila = fila.push(
                container(
                    text(c.to_string())
                        .size(14.0)
                        .color(color)
                        .wrapping(iced_core::text::Wrapping::None),
                )
                .padding(Padding {
                    top: (4.0 + dy).max(0.0),
                    bottom: (4.0 - dy).max(0.0),
                    ..Padding::ZERO
                }),
            );
        }
        fila.into()
    }

    /// Una tarjeta agrupada con sus filas y divisores con sangría de 16.
    // añadido en la 0.2.2
    fn grupo<'a>(p: &Paleta, filas: Vec<(PanelElement<'a>, bool)>) -> PanelElement<'a> {
        let mut col = column![];
        let div = p.div;
        for (i, (contenido, realzada)) in filas.into_iter().enumerate() {
            if i > 0 {
                col = col.push(
                    row![
                        Space::new().width(Length::Fixed(16.0)),
                        container(Space::new())
                            .width(Length::Fill)
                            .height(Length::Fixed(1.0))
                            .style(move |_: &iced_widget::Theme| container::Style {
                                background: Some(div.into()),
                                ..Default::default()
                            }),
                    ]
                    .height(Length::Fixed(1.0)),
                );
            }
            let fondo = realzada.then_some(p.realce);
            col = col.push(
                container(contenido)
                    .padding(Padding {
                        left: 16.0,
                        right: 14.0,
                        ..Padding::ZERO
                    })
                    .width(Length::Fill)
                    .height(Length::Fixed(if i > 0 { FILA - 1.0 } else { FILA }))
                    .align_y(Vertical::Center)
                    .style(move |_: &iced_widget::Theme| container::Style {
                        background: fondo.map(Into::into),
                        ..Default::default()
                    }),
            );
        }
        let fila = p.fila;
        container(col)
            .width(Length::Fill)
            .clip(true)
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(fila.into()),
                border: Border {
                    radius: GRUPO_RADIO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.0
    // modified 0.2.2 -- la mesa del lienzo, 1:1
    pub fn view(&self) -> PanelElement<'_> {
        let p = self.paleta();
        let ancho = ANCHO - 2.0 * LADO;
        let glifo = |nombre: &str, px: f32, color: Color| -> PanelElement<'static> {
            match icono::propio(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, px, color),
                None => Space::new().width(Length::Fixed(px)).into(),
            }
        };
        let redondo = |contenido: PanelElement<'static>, fondo: Color| -> PanelElement<'static> {
            container(contenido)
                .center_x(Length::Fixed(CIERRE))
                .center_y(Length::Fixed(CIERRE))
                .style(move |_: &iced_widget::Theme| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: (CIERRE / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
        };
        let etiqueta = |texto: &str| -> PanelElement<'static> {
            text(texto.to_owned())
                .size(14.0)
                .font(super::control::peso(iced_core::font::Weight::Medium))
                .color(p.tx)
                .into()
        };
        let valor = |texto: &str, sitio: f32| -> PanelElement<'static> {
            text(super::lista::recortar(texto, sitio, 14.0))
                .size(14.0)
                .color(p.tx2)
                .wrapping(iced_core::text::Wrapping::None)
                .into()
        };

        // Cerrar: círculo neutro, más oscuro bajo el puntero.
        let cerrar_señalado = self.señalada == Some(Zona::Cerrar);
        let cerrar = redondo(
            glifo("cerrar", 12.0, if cerrar_señalado { p.tx } else { p.tx2 }),
            if cerrar_señalado {
                tema::mezclar(p.fila, p.tx, 0.1)
            } else {
                p.fila
            },
        );

        // La ficha.
        let mut filas: Vec<(PanelElement<'_>, bool)> = self
            .datos
            .iter()
            .map(|d| {
                let sitio = ancho - 30.0 - d.etiqueta.chars().count() as f32 * 8.0 - 12.0;
                (
                    row![
                        etiqueta(d.etiqueta),
                        Space::new().width(Length::Fill),
                        valor(&d.valor, sitio)
                    ]
                    .align_y(Vertical::Center)
                    .into(),
                    false,
                )
            })
            .collect();
        let ojo_señalado = self.señalada == Some(Zona::Ojo);
        filas.push((
            row![
                etiqueta(t("Número de serie", "Serial number")),
                Space::new().width(Length::Fill),
                self.serie_animada(&p),
                Space::new().width(Length::Fixed(6.0)),
                redondo(
                    glifo(
                        if self.revelado { "ojo-tachado" } else { "ojo" },
                        16.0,
                        if ojo_señalado { p.tx } else { p.tx2 },
                    ),
                    if ojo_señalado { p.realce } else { p.fila },
                ),
            ]
            .align_y(Vertical::Center)
            .into(),
            false,
        ));

        // Las acciones como filas: una acción por fila (HIG §1).
        let copiado = self.copiado_desde.is_some_and(|d| d.elapsed() < COPIADO);
        let copiar: PanelElement<'_> = if copiado {
            row![
                glifo("comprobado", 14.0, tema::verde()),
                Space::new().width(Length::Fixed(8.0)),
                text(t("Ficha copiada", "Copied")).size(14.0).color(p.tx),
            ]
            .align_y(Vertical::Center)
            .into()
        } else {
            text(t("Copiar la ficha", "Copy details"))
                .size(14.0)
                .color(tema::enlace())
                .into()
        };
        let acciones = vec![
            (
                row![
                    etiqueta(t("Más información", "More info")),
                    Space::new().width(Length::Fill),
                    valor(t("Ajustes", "Settings"), 120.0),
                    Space::new().width(Length::Fixed(8.0)),
                    glifo("chevron-derecha", 12.0, p.tx2),
                ]
                .align_y(Vertical::Center)
                .into(),
                self.señalada == Some(Zona::Mas),
            ),
            (copiar, self.señalada == Some(Zona::Copiar)),
        ];

        let version = row![
            text(self.sistema.as_str())
                .size(13.0)
                .font(super::control::peso(iced_core::font::Weight::Semibold))
                .color(p.tx),
            text(format!(" · {}", self.base)).size(13.0).color(p.tx2),
        ];

        let contenido = column![
            // El retrato sube 6 bajo la altura del cierre, que va encima en
            // su capa: así el portátil no queda separado por una franja vacía.
            Space::new().height(Length::Fixed(CIERRE - 6.0)),
            self.retrato(),
            Space::new().height(Length::Fixed(14.0)),
            container(
                text(super::lista::recortar(&self.modelo, ancho, 22.0))
                    .size(22.0)
                    .font(super::control::peso(iced_core::font::Weight::Bold))
                    .color(p.tx)
                    .wrapping(iced_core::text::Wrapping::None)
            )
            .center_x(Length::Fill)
            .height(Length::Fixed(NOMBRE)),
            Space::new().height(Length::Fixed(2.0)),
            container(version)
                .center_x(Length::Fill)
                .height(Length::Fixed(VERSION)),
            Space::new().height(Length::Fixed(HUECO)),
            Self::grupo(&p, filas),
            Space::new().height(Length::Fixed(HUECO)),
            Self::grupo(&p, acciones),
        ];

        let (_, alto) = self.size();
        let fondo = p.fondo;
        container(stack![
            container(contenido).padding(Padding {
                top: ARRIBA,
                right: LADO,
                bottom: ABAJO,
                left: LADO,
            }),
            container(cerrar)
                .width(Length::Fill)
                .align_x(Horizontal::Right)
                .padding(Padding {
                    top: ARRIBA,
                    right: LADO,
                    ..Padding::ZERO
                }),
        ])
        .width(Length::Fixed(ANCHO))
        .height(Length::Fixed(alto))
        .clip(true)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_DIALOGO.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }
}

/// De 0 a 1, cuánto lleva una animación que empieza `retraso` después del
/// clic y dura `dura`.
// añadido en la 0.2.2
fn fraccion(pasado: Duration, retraso: Duration, dura: Duration) -> f32 {
    (pasado.saturating_sub(retraso).as_secs_f32() / dura.as_secs_f32()).clamp(0.0, 1.0)
}

/// Los colores del diálogo; ver [`Acerca::paleta`].
// añadido en la 0.2.2
#[derive(Clone, Copy)]
struct Paleta {
    fondo: Color,
    fila: Color,
    tx: Color,
    tx2: Color,
    div: Color,
    realce: Color,
}

/// El disco donde está `/`: su tamaño de fábrica (en potencias de 10, como lo
/// anuncia el fabricante: 1 TB y no 931 GiB) y lo libre para el usuario.
// añadido en la 0.2.2
fn almacenamiento() -> Option<String> {
    let total = tamaño_del_disco()?;
    // SAFETY: `statvfs` solo escribe en la estructura que se le pasa, y la
    // ruta es un C string literal terminado en cero.
    let libre = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(c"/".as_ptr(), &mut st) == 0)
            .then(|| st.f_bavail as u64 * st.f_frsize as u64)
    }?;
    Some(format!(
        "{} · {} {}",
        bytes(total),
        bytes(libre),
        t("libres", "free")
    ))
}

/// El tamaño del primer disco fijo, de `/sys/block/*/size` (sectores de 512).
/// Se toma el mayor de los no extraíbles: el del sistema en un portátil.
// añadido en la 0.2.2
fn tamaño_del_disco() -> Option<u64> {
    std::fs::read_dir("/sys/block")
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !(n.starts_with("loop")
                || n.starts_with("zram")
                || n.starts_with("dm-")
                || n.starts_with("ram"))
        })
        .filter(|e| {
            std::fs::read_to_string(e.path().join("removable")).is_ok_and(|r| r.trim() == "0")
        })
        .filter_map(|e| std::fs::read_to_string(e.path().join("size")).ok())
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .map(|sectores| sectores * 512)
        .max()
}

/// Bytes en GB o TB decimales, con coma: «1 TB», «812 GB», «1,4 TB».
// añadido en la 0.2.2
fn bytes(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 1000.0 {
        let tb = gb / 1000.0;
        if (tb - tb.round()).abs() < 0.05 {
            format!("{:.0} TB", tb.round())
        } else {
            format!("{:.1} TB", tb).replace('.', ",")
        }
    } else {
        format!("{:.0} GB", gb)
    }
}

/// La pantalla interna: las pulgadas de su EDID (ancho y alto en cm, bytes
/// 21 y 22) y su resolución nativa, el primer modo que anuncia.
// añadido en la 0.2.2
fn pantalla() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().contains("-eDP-"))
        })?;
    let edid = std::fs::read(dir.join("edid")).ok()?;
    let (w, h) = (*edid.get(21)? as f32, *edid.get(22)? as f32);
    let modo = std::fs::read_to_string(dir.join("modes")).ok()?;
    let modo = modo.lines().next()?.replace('x', " × ");
    if w == 0.0 || h == 0.0 {
        return Some(modo);
    }
    let pulgadas = (w * w + h * h).sqrt() / 2.54;
    Some(format!(
        "{} {} · {modo}",
        pulgadas.round(),
        t("pulgadas", "inches")
    ))
}

/// Un campo de DMI. Vacío o ilegible cuenta como que no está: `product_serial`
/// es 0400 de root y siempre falla, y eso no es un error que reportar.
/// El dibujo del portátil que corresponde a `modelo` (`product_family` de
/// DMI). El 360 se mira antes porque su nombre contiene el del Book5 Pro.
/// Cualquier otro equipo lleva el de frente, que no se parece a ninguno.
// añadido en la 0.2.0
// modified 0.2.2 -- el Book5 Pro con el render de Blender, no el SVG
fn dibujo_del_modelo(modelo: &str) -> Dibujo {
    let vector = |bytes: &'static [u8]| Dibujo::Vector(svg::Handle::from_memory(bytes));
    if modelo.contains("Book5 Pro 360") {
        vector(include_bytes!("../../assets/portatil-book5-pro-360.svg"))
    } else if modelo.contains("Book5 Pro") {
        // `render-book5-recto/render-verde.png` (4000×2480) con el escritorio azul
        // por defecto en el hueco verde de la pantalla, recortado al portátil,
        // sin el ruido de alfa que dejaba el fondo del render, y a 510 px de
        // alto: el retrato mide 170, así que sale nítido hasta escala 3.
        let png = include_bytes!("../../assets/portatil-book5-pro.png");
        let rgba = image::load_from_memory(png)
            .expect("el PNG del render va dentro del binario")
            .to_rgba8();
        let (w, h) = rgba.dimensions();
        Dibujo::Render(iced_widget::image::Handle::from_rgba(w, h, rgba.into_raw()))
    } else {
        vector(include_bytes!("../../assets/portatil.svg"))
    }
}

/// El portátil de arriba: un SVG o el render. Se crea una vez: los dos
/// `Handle` estrenan identificador al crearse y se rasterizarían en cada
/// fotograma.
// añadido en la 0.2.2
enum Dibujo {
    Vector(svg::Handle),
    Render(iced_widget::image::Handle),
}

// 0.1.0
fn dmi(campo: &str) -> Option<String> {
    let valor = std::fs::read_to_string(format!("/sys/class/dmi/id/{campo}")).ok()?;
    let valor = valor.trim();
    (!valor.is_empty()).then(|| valor.to_string())
}

// 0.1.0
fn os_release(clave: &str) -> Option<String> {
    let texto = std::fs::read_to_string("/etc/os-release").ok()?;
    for linea in texto.lines() {
        let Some((k, v)) = linea.split_once('=') else {
            continue;
        };
        if k == clave {
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}

/// El modelo de la CPU, sin las marcas registradas: «Intel(R) Core(TM) Ultra 7»
/// con sus símbolos ocupa media línea y no dice nada más.
// 0.1.0
fn cpu() -> Option<String> {
    let texto = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let linea = texto.lines().find(|l| l.starts_with("model name"))?;
    let modelo = linea.split_once(':')?.1.trim();
    Some(
        modelo
            .replace("(R)", "")
            .replace("(TM)", "")
            .replace("  ", " "),
    )
}

/// La memoria en GB redondeados.
///
/// `MemTotal` no es la memoria instalada sino la que ve el kernel —descontando
/// lo que se reserva el firmware—, así que 32 GiB salen como 30,8. Se redondea
/// al múltiplo de 4 más cercano, que es lo que de verdad hay puesto.
// 0.1.0
fn memoria() -> Option<String> {
    let texto = std::fs::read_to_string("/proc/meminfo").ok()?;
    let linea = texto.lines().find(|l| l.starts_with("MemTotal"))?;
    let kb: f64 = linea.split_whitespace().nth(1)?.parse().ok()?;
    let gb = kb / 1024.0 / 1024.0;
    let redondeado = (gb / 4.0).round() * 4.0;
    Some(format!("{redondeado:.0} GB"))
}

/// El nombre comercial de la tarjeta gráfica.
// 0.1.0
fn gpu() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/drm").ok()?;
    for entrada in dir.filter_map(|e| e.ok()) {
        let nombre = entrada.file_name().to_string_lossy().to_string();
        // Solo las tarjetas, no los conectores (`card1-eDP-1`).
        if !nombre.starts_with("card") || nombre.contains('-') {
            continue;
        }
        let dispositivo = entrada.path().join("device");
        let leer = |f: &str| {
            std::fs::read_to_string(dispositivo.join(f))
                .ok()
                .map(|s| s.trim().trim_start_matches("0x").to_lowercase())
        };
        let (Some(vendor), Some(device)) = (leer("vendor"), leer("device")) else {
            continue;
        };
        if let Some(nombre) = buscar_pci(&vendor, &device) {
            return Some(acortar_gpu(&nombre));
        }
    }
    None
}

/// Deja el nombre de la tarjeta en algo que quepa en un renglón.
///
/// `pci.ids` los escribe para un catálogo, no para una ficha: esta sale como
/// "Core Ultra 200V Series Processors Arc Graphics 130V/140V GPU", que ocupaba
/// tres líneas y se comía el renglón de debajo. Se quitan las palabras de
/// relleno y, si aparece una familia conocida —`Arc`, `Radeon`, `GeForce`—, se
/// corta por ahí, que es donde empieza lo que identifica a la tarjeta.
// 0.1.0
fn acortar_gpu(nombre: &str) -> String {
    const RELLENO: &[&str] = &[
        " Series Processors",
        " Integrated Graphics Controller",
        " Corporation",
        " GPU",
        " Controller",
    ];
    let mut corto = nombre.to_string();
    for palabra in RELLENO {
        corto = corto.replace(palabra, "");
    }
    for familia in ["Arc ", "Radeon ", "GeForce "] {
        if let Some(i) = corto.find(familia) {
            corto = corto[i..].to_string();
            break;
        }
    }
    corto.trim().to_string()
}

/// Traduce `vendor:device` con la tabla de hwdata, que viene con el sistema.
///
/// El fichero son 1,6 MB y se recorre entero en el peor caso; se hace **una
/// vez**, al abrir esta ventana, y nunca desde el camino del panel.
// 0.1.0
fn buscar_pci(vendor: &str, device: &str) -> Option<String> {
    let texto = std::fs::read_to_string("/usr/share/hwdata/pci.ids").ok()?;
    let mut en_vendor = false;
    for linea in texto.lines() {
        if linea.starts_with('#') || linea.trim().is_empty() {
            continue;
        }
        if !linea.starts_with('\t') {
            // Sale del bloque del fabricante: si ya estábamos dentro, el
            // dispositivo no aparece y no hace falta leer el resto.
            if en_vendor {
                return None;
            }
            en_vendor = linea.starts_with(vendor);
            continue;
        }
        // Las subclases llevan dos tabuladores; los dispositivos, uno.
        if !en_vendor || linea.starts_with("\t\t") {
            continue;
        }
        let linea = linea.trim_start();
        let Some((id, nombre)) = linea.split_once(' ') else {
            continue;
        };
        if id == device {
            let mut salida = String::new();
            let _ = write!(salida, "{}", nombre.trim());
            return Some(salida);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_360_no_se_toma_por_el_book5_pro() {
        let book5 = dibujo_del_modelo("Galaxy Book5 Pro");
        let convertible = dibujo_del_modelo("Galaxy Book5 Pro 360");
        let otro = dibujo_del_modelo("ThinkPad X1");
        assert!(
            matches!(book5, Dibujo::Render(_)),
            "el Book5 Pro lleva el render"
        );
        let (Dibujo::Vector(convertible), Dibujo::Vector(otro)) = (convertible, otro) else {
            panic!("el 360 y los demás siguen en SVG");
        };
        // El id de un SVG es el hash de su contenido: distinto id, distinto dibujo.
        assert_ne!(convertible.id(), otro.id());
    }

    /// Los datos salen de esta máquina, así que lo que se comprueba es que
    /// **hay** ficha y que ningún renglón queda a medias.
    #[test]
    fn la_ficha_no_tiene_renglones_vacios() {
        let a = Acerca::new();
        assert_eq!(a.datos.len(), 5);
        for dato in &a.datos {
            assert!(!dato.valor.trim().is_empty(), "{} sin valor", dato.etiqueta);
        }
        assert!(!a.modelo.is_empty());
    }

    /// El número sale tapado al abrir y el ojo —toda su fila— lo alterna.
    #[test]
    fn el_ojo_alterna_el_numero_de_serie() {
        let mut a = Acerca::new();
        assert!(!a.revelado);
        let ojo = a.rect(Zona::Ojo).center();
        assert_eq!(a.pulsar(ojo.x, ojo.y), None);
        assert!(a.revelado);
        a.pulsar(ojo.x, ojo.y);
        assert!(!a.revelado);
    }

    /// Cerrar cierra, y ninguna zona pisa a otra ni se sale de la tarjeta.
    #[test]
    fn las_zonas_no_se_pisan_y_caben() {
        let mut a = Acerca::new();
        let c = a.rect(Zona::Cerrar).center();
        assert_eq!(a.pulsar(c.x, c.y), Some(Accion::CerrarEmergente));
        let zonas = [Zona::Cerrar, Zona::Ojo, Zona::Mas, Zona::Copiar];
        let (w, h) = a.size();
        for (i, z) in zonas.iter().enumerate() {
            let r = a.rect(*z);
            assert!(
                r.y + r.height <= h - ABAJO + 0.5 && r.x + r.width <= w,
                "{z:?} se sale"
            );
            for otra in &zonas[i + 1..] {
                // Cerrar lleva 6 px de holgura que pueden rozar el aire del
                // retrato, nunca otra zona.
                assert!(
                    r.intersection(&a.rect(*otra)).is_none(),
                    "{z:?} pisa {otra:?}"
                );
            }
        }
    }

    /// «Copiar la ficha» manda el texto y deja la ventana abierta; el número
    /// de serie solo va si está a la vista.
    #[test]
    fn copiar_la_ficha() {
        let mut a = Acerca::new();
        a.serie = Some("R9NX40A1B2C".into());
        let c = a.rect(Zona::Copiar).center();
        let Some(Accion::CopiarTexto(tapada)) = a.pulsar(c.x, c.y) else {
            panic!("copiar manda texto");
        };
        assert!(tapada.starts_with(&a.modelo));
        assert!(tapada.contains("BookUI "));
        assert!(!tapada.contains("R9NX40A1B2C"), "tapado no se copia");
        a.revelado = true;
        let Some(Accion::CopiarTexto(vista)) = a.pulsar(c.x, c.y) else {
            panic!("copiar manda texto");
        };
        assert!(vista.contains("R9NX40A1B2C"));
        assert!(
            Accion::CopiarTexto(vista).conserva_emergente(),
            "no cierra la ventana"
        );
    }

    /// El subtítulo es el escritorio, BookUI, con la base detrás en corto.
    #[test]
    fn el_subtitulo_es_bookui_y_la_base_va_aparte() {
        let a = Acerca::new();
        assert!(a.sistema.starts_with("BookUI "), "subtítulo: {}", a.sistema);
        assert!(!a.sistema.to_lowercase().contains("fedora"));
        assert!(!a.base.is_empty());
        assert!(!a.base.contains("KDE"), "la base va corta: {}", a.base);
    }

    #[test]
    fn los_bytes_salen_como_los_anuncia_el_fabricante() {
        assert_eq!(bytes(1_000_204_886_016), "1 TB");
        assert_eq!(bytes(812_000_000_000), "812 GB");
        assert_eq!(bytes(1_400_000_000_000), "1,4 TB");
    }

    /// `BOOKOS_ACERCA_PNG=/tmp/a.png` vuelca la tarjeta en oscuro y en claro.
    #[test]
    fn volcar_png() {
        let Ok(ruta) = std::env::var("BOOKOS_ACERCA_PNG") else {
            return;
        };
        for (sufijo, tema_) in [("oscuro", tema::Tema::Oscuro), ("claro", tema::Tema::Claro)] {
            tema::aplicar(tema_);
            let mut a = Acerca::new();
            a.revelado = sufijo == "claro";
            let config = crate::Config {
                tema: tema_,
                ..Default::default()
            };
            let mut shell = crate::Shell::con_config(1646, 1.0, config);
            shell.abrir(super::super::Emergente::Acerca(a));
            let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_emergente(&mut buf);
            for p in buf.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            image::save_buffer(
                ruta.replace(".png", &format!("-{sufijo}.png")),
                &buf,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .expect("se escribe el PNG");
        }
    }

    /// La memoria se redondea a lo que de verdad hay puesto: el kernel
    /// descuenta lo que se reserva el firmware.
    #[test]
    fn la_memoria_sale_redondeada() {
        let Some(m) = memoria() else {
            return;
        };
        let gb: f64 = m.trim_end_matches(" GB").parse().expect("memoria en GB");
        assert_eq!(gb % 4.0, 0.0, "{m} no es múltiplo de 4");
    }
}
