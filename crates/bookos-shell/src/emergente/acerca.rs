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

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length, Point, Rectangle};
use iced_widget::{Space, column, container, row, stack, svg, text};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, Tecla, t};

const ANCHO: f32 = 420.0;
const MARGEN: f32 = 32.0;
/// Alto del dibujo del portátil de arriba.
const RETRATO: f32 = 170.0;
/// Proporción de `portatil.svg` (380×217), el más ancho de los tres: los de
/// la serie Book5 Pro, más altos, caben en la misma caja centrados.
const RETRATO_ANCHO: f32 = RETRATO * 380.0 / 217.0;
/// Alto del nombre del equipo y del renglón de BookOS bajo él. Fijos y no
/// a lo que mida el texto: la ficha de debajo tiene que empezar siempre en el
/// mismo sitio para que el ojo caiga donde lo busca [`Acerca::rect_ojo`].
const TITULO: f32 = 40.0;
const SUBTITULO: f32 = 20.0;
/// Alto de cada renglón de la ficha, contando su respiro.
const RENGLON: f32 = 26.0;
/// Una línea más para el último renglón, «Base», que es el único que puede
/// partirse en dos: "Fedora Linux 44 (KDE Plasma Desktop Edition)" no cabe.
const LINEA_EXTRA: f32 = 18.0;
/// Ancho de la columna de las etiquetas. Van alineadas a la derecha contra el
/// valor, como en la ficha del diseño.
const ETIQUETAS: f32 = 160.0;
/// Hueco entre etiqueta y valor.
const SEPARACION: f32 = 12.0;
/// Alto del botón de «Más información…».
const BOTON: f32 = 44.0;
const ANCHO_BOTON: f32 = 264.0;
/// Lado del ojo y de la ✕.
const OJO: f32 = 18.0;
const CIERRE: f32 = 24.0;
/// Distancia de la ✕ a las esquinas de la tarjeta: con el radio de 22 de la
/// tarjeta, a menos de 16 la ✕ pisa la curva.
const CIERRE_MARGEN: f32 = 16.0;
/// En qué renglón de la ficha va el número de serie.
const RENGLON_SERIE: usize = 3;
/// Lo que se enseña mientras el número está tapado. Siempre la misma
/// cantidad de puntos: si dependiera del número, ya diría cuánto mide.
const TAPADO: &str = "••••••••••";

/// Un renglón de la ficha.
struct Dato {
    etiqueta: &'static str,
    valor: String,
}

pub struct Acerca {
    /// Cómo se llama el equipo: «Galaxy Book5 Pro».
    modelo: String,
    /// La distribución, para el renglón bajo el título.
    sistema: String,
    datos: Vec<Dato>,
    /// `None` si no se pudo leer: falta la regla de udev o no se es de `wheel`.
    serie: Option<String>,
    revelado: bool,
    /// Se crea una vez: `Handle::from_memory` estrena identificador en cada
    /// llamada y resvg volvería a rasterizar el dibujo en cada fotograma.
    portatil: svg::Handle,
    /// Si el ratón está sobre el botón de «Más información…».
    /// El botón bajo el puntero, con su realce entrando y saliendo.
    señalado: tema::Transicion,
}

impl Acerca {
    // 0.1.0
    // modificado en la 0.2.0 -- el portátil según el modelo
    pub fn new() -> Self {
        let modelo = dmi("product_family")
            .or_else(|| dmi("product_name"))
            .unwrap_or_else(|| t("Este equipo", "This computer").into());
        // El subtítulo dice **BookOS**, no la distribución de base. BookOS no
        // es una distro aparte —es un escritorio que corre sobre Fedora—, así
        // que `/etc/os-release` describe el sistema anfitrión, no lo que el
        // usuario está viendo en pantalla; enseñar "Fedora Linux 44 (KDE
        // Plasma Desktop Edition)" ahí es tan equivocado como que un Mac
        // dijera "Darwin" en su «Acerca de este Mac». La versión sale del
        // paquete del propio compositor, que es la única versión de BookOS
        // que existe todavía.
        let sistema = format!("BookOS {}", env!("CARGO_PKG_VERSION"));
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
            // Destapado, `ficha` pone el número en su lugar.
            Dato {
                etiqueta: t("Número de serie", "Serial number"),
                valor: TAPADO.into(),
            },
            Dato {
                // Aquí sí es sitio para la base: a quien le importe qué hay
                // debajo del escritorio, aquí lo tiene completo y sin recortar
                // —ya no compite con el subtítulo por decir lo mismo.
                etiqueta: "Base",
                valor: os_release("PRETTY_NAME").unwrap_or_else(|| "Linux".into()),
            },
        ];
        let portatil = svg::Handle::from_memory(dibujo_del_modelo(&modelo));
        Self {
            modelo,
            sistema,
            datos,
            serie: dmi("product_serial"),
            revelado: false,
            portatil,
            señalado: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (
            ANCHO,
            self.y_ficha()
                + self.datos.len() as f32 * RENGLON
                + LINEA_EXTRA
                + 28.0
                + BOTON
                + MARGEN,
        )
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// Dónde empieza la ficha, relativo a la tarjeta.
    // 0.1.0
    fn y_ficha(&self) -> f32 {
        MARGEN + RETRATO + 24.0 + TITULO + SUBTITULO + 24.0
    }

    /// El botón va pegado al fondo de la tarjeta, así que su sitio no depende
    /// de si «Base» ocupa una línea o dos.
    // 0.1.0
    fn rect_boton(&self) -> Rectangle {
        Rectangle {
            x: (ANCHO - ANCHO_BOTON) / 2.0,
            y: self.size().1 - MARGEN - BOTON,
            width: ANCHO_BOTON,
            height: BOTON,
        }
    }

    /// Todo el valor del número de serie, no solo el ojo: el ojo mide 18 px y
    /// pulsar sobre los puntos para destaparlos es lo que se intenta primero.
    // 0.1.0
    fn rect_ojo(&self) -> Rectangle {
        let x = MARGEN + ETIQUETAS;
        Rectangle {
            x,
            y: self.y_ficha() + RENGLON_SERIE as f32 * RENGLON,
            width: ANCHO - MARGEN - x,
            height: RENGLON,
        }
    }

    // 0.1.0
    fn rect_cierre(&self) -> Rectangle {
        Rectangle {
            x: ANCHO - CIERRE_MARGEN - CIERRE,
            y: CIERRE_MARGEN,
            width: CIERRE,
            height: CIERRE,
        }
        // Un poco de holgura alrededor: 24 px en una esquina se fallan.
        .expand(6.0)
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let dentro = punto.is_some_and(|(x, y)| self.rect_boton().contains(Point::new(x, y)));
        self.señalado.ir_a(dentro as u8 as f32)
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalado.animando()
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = Point::new(x, y);
        if self.rect_cierre().contains(punto) {
            return Some(Accion::CerrarEmergente);
        }
        if self.rect_ojo().contains(punto) {
            self.revelado = !self.revelado;
            return None;
        }
        if !self.rect_boton().contains(punto) {
            return None;
        }
        // «Más información…» abre «Acerca del portátil» en los ajustes. Antes
        // se pasaba `--acerca`, que bookos-settings no conoce: arrancaba sin
        // error en su portada y el `||` de respaldo nunca saltaba.
        Some(Accion::Lanzar("bookos-settings --page acerca".into()))
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// El velo de detrás: la tarjeta ocupa el centro de la pantalla y sin él se
    /// leería como una ventana más del escritorio.
    // 0.1.0
    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..tema::bg()
        }
    }

    /// El portátil de arriba, el del diseño.
    // 0.1.0
    fn retrato(&self) -> PanelElement<'_> {
        container(
            svg(self.portatil.clone())
                .width(Length::Fixed(RETRATO_ANCHO))
                .height(Length::Fixed(RETRATO)),
        )
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .into()
    }

    /// El valor del número de serie con su ojo detrás.
    // 0.1.0
    fn serie(&self) -> PanelElement<'_> {
        let (valor, ojo) = match (self.revelado, &self.serie) {
            (false, _) => (TAPADO, "ojo"),
            (true, Some(serie)) => (serie.as_str(), "ojo-tachado"),
            (true, None) => (
                t("Sin permiso para leerlo", "No permission to read it"),
                "ojo-tachado",
            ),
        };
        let ojo = match icono::propio(ojo) {
            Some(ic) => icono::ver_teñido_propio(&ic, OJO, tema::TEXTO2),
            None => Space::new().into(),
        };
        row![
            text(valor).size(tema::T_CUERPO).color(tema::TEXTO2),
            Space::new().width(Length::Fixed(10.0)),
            ojo,
        ]
        .align_y(Vertical::Center)
        .into()
    }

    // 0.1.0
    fn ficha(&self) -> PanelElement<'_> {
        let mut col = column![];
        let ultimo = self.datos.len() - 1;
        for (i, dato) in self.datos.iter().enumerate() {
            let valor: PanelElement<'_> = if i == RENGLON_SERIE {
                self.serie()
            } else {
                text(dato.valor.as_str())
                    .size(tema::T_CUERPO)
                    .color(tema::TEXTO2)
                    .into()
            };
            let renglon = container(
                row![
                    container(
                        text(dato.etiqueta)
                            .size(tema::T_CUERPO)
                            .color(tema::texto())
                            .align_x(Horizontal::Right),
                    )
                    .width(Length::Fixed(ETIQUETAS))
                    .align_x(Horizontal::Right),
                    Space::new().width(Length::Fixed(SEPARACION)),
                    valor,
                ]
                .align_y(Vertical::Center),
            )
            .width(Length::Fill);
            // Alto fijo en todos menos en el último, que puede partirse en dos
            // líneas sin mover nada que se pulse: el ojo va encima de él.
            col = col.push(if i == ultimo {
                renglon.padding(iced_core::Padding::ZERO.top(4.0))
            } else {
                renglon
                    .height(Length::Fixed(RENGLON))
                    .align_y(Vertical::Center)
            });
        }
        col.into()
    }

    /// La ✕ roja de la esquina, como la del Launchpad.
    // 0.1.0
    fn cierre(&self) -> PanelElement<'_> {
        let aspa: PanelElement<'_> = match icono::propio("cerrar") {
            Some(ic) => {
                icono::ver_teñido_propio(&ic, CIERRE * 0.7, tema::tinta_sobre(tema::rojo()))
            }
            None => Space::new().into(),
        };
        container(
            container(aspa)
                .center_x(Length::Fixed(CIERRE))
                .center_y(Length::Fixed(CIERRE))
                .style(|_theme: &iced_widget::Theme| container::Style {
                    background: Some(tema::rojo().into()),
                    border: Border {
                        radius: tema::R_CHIP.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fill)
        .align_x(Horizontal::Right)
        .padding(CIERRE_MARGEN as u16)
        .into()
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let boton = container(
            container(
                text(t("Más información", "More info"))
                    .size(tema::T_TITULO)
                    .color(tema::texto()),
            )
            .center_x(Length::Fixed(ANCHO_BOTON))
            .center_y(Length::Fixed(BOTON))
            .style({
                let señalado = self.señalado.valor();
                move |_theme: &iced_widget::Theme| container::Style {
                    background: Some(
                        tema::mezclar(
                            tema::alfa(tema::tinta(), 0.28),
                            tema::alfa(tema::tinta(), 0.36),
                            señalado,
                        )
                        .into(),
                    ),
                    border: Border {
                        radius: tema::R_BOTON.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
        )
        .width(Length::Fill)
        .align_x(Horizontal::Center);

        let contenido = column![
            self.retrato(),
            Space::new().height(Length::Fixed(24.0)),
            container(text(self.modelo.as_str()).size(30.0).color(tema::texto()))
                .center_x(Length::Fill)
                .height(Length::Fixed(TITULO)),
            container(
                text(self.sistema.as_str())
                    .size(tema::T_CUERPO)
                    .color(tema::TEXTO2)
            )
            .center_x(Length::Fill)
            .height(Length::Fixed(SUBTITULO)),
            Space::new().height(Length::Fixed(24.0)),
            self.ficha(),
            // Lo que sobre si «Base» cabe en una línea va aquí, entre la ficha
            // y el botón, que así queda siempre donde lo busca `rect_boton`.
            Space::new().height(Length::Fill),
            boton,
        ];

        let (_, alto) = self.size();
        container(stack![
            container(contenido)
                .padding(MARGEN as u16)
                .height(Length::Fixed(alto)),
            self.cierre(),
        ])
        .width(Length::Fixed(ANCHO))
        .height(Length::Fixed(alto))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                width: 1.0,
                color: tema::borde(),
            },
            ..Default::default()
        })
        .into()
    }
}

/// Un campo de DMI. Vacío o ilegible cuenta como que no está: `product_serial`
/// es 0400 de root y siempre falla, y eso no es un error que reportar.
/// El dibujo del portátil que corresponde a `modelo` (`product_family` de
/// DMI). El 360 se mira antes porque su nombre contiene el del Book5 Pro.
/// Cualquier otro equipo lleva el de frente, que no se parece a ninguno.
// añadido en la 0.2.0
fn dibujo_del_modelo(modelo: &str) -> &'static [u8] {
    if modelo.contains("Book5 Pro 360") {
        include_bytes!("../../assets/portatil-book5-pro-360.svg")
    } else if modelo.contains("Book5 Pro") {
        include_bytes!("../../assets/portatil-book5-pro.svg")
    } else {
        include_bytes!("../../assets/portatil.svg")
    }
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
        assert_ne!(book5, convertible);
        assert_ne!(book5, otro);
        assert_ne!(convertible, otro);
    }

    /// Los datos salen de esta máquina, así que lo que se comprueba es que
    /// **hay** ficha y que ningún renglón queda a medias: un valor vacío se ve
    /// como un fallo de dibujado, no como un dato que falta.
    #[test]
    fn la_ficha_no_tiene_renglones_vacios() {
        let a = Acerca::new();
        assert_eq!(a.datos.len(), 5);
        for dato in &a.datos {
            assert!(!dato.valor.trim().is_empty(), "{} sin valor", dato.etiqueta);
        }
        assert!(!a.modelo.is_empty());
    }

    /// El número sale tapado al abrir y el ojo lo alterna. Sin permiso de
    /// lectura —lo normal en un test, que no corre como `wheel` con la regla
    /// puesta— destapa el aviso, nunca un renglón vacío.
    #[test]
    fn el_ojo_alterna_el_numero_de_serie() {
        let mut a = Acerca::new();
        assert!(!a.revelado);
        let ojo = a.rect_ojo().center();
        assert_eq!(a.pulsar(ojo.x, ojo.y), None);
        assert!(a.revelado);
        a.pulsar(ojo.x, ojo.y);
        assert!(!a.revelado);
    }

    #[test]
    fn la_aspa_cierra_y_el_boton_no_se_solapa_con_ella() {
        let mut a = Acerca::new();
        let c = a.rect_cierre().center();
        assert_eq!(a.pulsar(c.x, c.y), Some(Accion::CerrarEmergente));
        assert!(a.rect_boton().intersection(&a.rect_ojo()).is_none());
    }

    /// El subtítulo dice BookOS, no la distribución de base. Es justo el
    /// bug que se reportó: la ficha enseñaba «Fedora Linux 44 (KDE Plasma
    /// Desktop Edition)», que es verdad de la máquina pero no de lo que el
    /// usuario tiene delante.
    #[test]
    fn el_subtitulo_es_bookos_y_no_la_distro_de_base() {
        let a = Acerca::new();
        assert!(a.sistema.starts_with("BookOS "), "subtítulo: {}", a.sistema);
        assert!(!a.sistema.to_lowercase().contains("fedora"));
        let base = a
            .datos
            .iter()
            .find(|d| d.etiqueta == "Base")
            .expect("hay un renglón de base");
        // La base sí puede —y en esta máquina, debe— decir Fedora: ese
        // renglón es justo el sitio para la verdad incómoda que no pertenece
        // al título.
        assert!(!base.valor.is_empty());
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
