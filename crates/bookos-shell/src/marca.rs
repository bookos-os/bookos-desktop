//! El logo de BookOS.
//!
//! **Va dentro del binario**, no en `/usr/share`. El panel tiene que pintarse en
//! el primer frame sin depender de que nadie haya instalado nada, y el logo es
//! lo primero que se ve del escritorio: buscarlo en disco añade un camino en el
//! que puede no aparecer, y un panel al que le falta su propia marca parece un
//! escritorio a medio instalar.
//!
//! Son 7 KB de SVG. resvg lo rasteriza al tamaño que se pida, así que el mismo
//! fichero sirve para los 20 px del panel y para los 96 del "Acerca de".
//!
//! **El color dice el canal de actualizaciones**: lila en estable, naranja en
//! beta y rojo en dev. Los dibujos del diseño para beta y dev son la misma
//! marca con otros dos colores, así que se recolorea el SVG en vez de llevar
//! tres PNG de 128 px que a 20 px se verían borrosos.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use iced_widget::svg;

/// El SVG completo, tal cual sale del diseño: el círculo, la onda y el nombre.
const LOGO: &str = include_str!("../assets/bookos.svg");

/// El mismo logo **sin las letras**, para tamaños pequeños.
///
/// No es un capricho: el nombre va dibujado dentro del círculo y a los 20 px
/// del panel se convierte en un borrón gris de dos líneas. Se comprobó
/// pintándolo. A partir de unos 64 px el logo completo ya se lee, y ahí se usa
/// el otro.
const MARCA: &str = include_str!("../assets/bookos-marca.svg");

/// A partir de qué tamaño el nombre de dentro del logo se lee.
const LEGIBLE: f32 = 64.0;

/// El canal de actualizaciones elegido en Ajustes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Canal {
    Estable,
    Beta,
    Dev,
}

impl Canal {
    const TODOS: [Canal; 3] = [Canal::Estable, Canal::Beta, Canal::Dev];

    /// El canal como lo resuelve BookOS Settings (`get_update_channel`):
    /// `~/.config/bookos-update.conf` manda sobre `/etc/bookos-update.conf`
    /// —Settings escribe en el de casa cuando no puede en `/etc`—, después
    /// `/etc/bookos-release`, y sin nada, estable.
    // 0.1.0
    pub fn leer() -> Self {
        let casa = std::env::var_os("HOME")
            .map(|h| std::path::Path::new(&h).join(".config/bookos-update.conf"));
        casa.into_iter()
            .chain(["/etc/bookos-update.conf", "/etc/bookos-release"].map(std::path::PathBuf::from))
            .find_map(|ruta| {
                let texto = std::fs::read_to_string(ruta).ok()?;
                texto.lines().find_map(|l| {
                    let (clave, valor) = l.split_once('=')?;
                    (clave.trim() == "CHANNEL").then(|| Self::de_texto(valor))?
                })
            })
            .unwrap_or(Canal::Estable)
    }

    // 0.1.0
    fn de_texto(valor: &str) -> Option<Self> {
        match valor.trim().trim_matches('"') {
            "stable" => Some(Canal::Estable),
            "beta" => Some(Canal::Beta),
            "dev" => Some(Canal::Dev),
            _ => None,
        }
    }

    /// El círculo y la onda. Sacados de los dibujos del diseño
    /// (`bookos-{stable,beta,dev}.png`); los de estable son los del SVG.
    // 0.1.0
    fn colores(self) -> (&'static str, &'static str) {
        match self {
            Canal::Estable => ("#A4BBFE", "#7E9FFF"),
            Canal::Beta => ("#CA6F2A", "#A64A03"),
            Canal::Dev => ("#CA2A2A", "#A40F0F"),
        }
    }

    // 0.1.0
    fn indice(self) -> usize {
        self as usize
    }
}

static CANAL: AtomicU8 = AtomicU8::new(0);

/// Pone el canal del proceso. Quien lo llame tiene que repintar el panel.
// 0.1.0
pub fn aplicar_canal(canal: Canal) {
    CANAL.store(canal.indice() as u8, Ordering::Relaxed);
}

// 0.1.0
pub fn canal_actual() -> Canal {
    Canal::TODOS[CANAL.load(Ordering::Relaxed) as usize]
}

/// `(logo, marca)` de cada canal, hechos una vez: `Handle::from_memory`
/// estrena identificador en cada llamada y resvg volvería a rasterizar el
/// logo en cada fotograma del panel.
static DIBUJOS: LazyLock<[(svg::Handle, svg::Handle); 3]> = LazyLock::new(|| {
    Canal::TODOS.map(|canal| {
        let (circulo, onda) = canal.colores();
        let pintar = |svg: &str| {
            svg::Handle::from_memory(
                svg.replace("#A4BBFE", circulo)
                    .replace("#7E9FFF", onda)
                    .into_bytes(),
            )
        };
        (pintar(LOGO), pintar(MARCA))
    })
});

/// El logo al tamaño que se pida, sin teñir.
///
/// A diferencia de los iconos de estado, este **no** se colorea: tiene su
/// propia paleta —el círculo, la onda y el nombre en blanco— y pintarlo de un
/// solo color lo convertiría en una mancha redonda.
// 0.1.0
pub fn ver<'a>(px: f32) -> crate::view::PanelElement<'a> {
    use iced_core::Length;
    let (logo, marca) = &DIBUJOS[canal_actual().indice()];
    let handle = if px >= LEGIBLE { logo } else { marca };
    svg(handle.clone())
        .width(Length::Fixed(px))
        .height(Length::Fixed(px))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Si el SVG del diseño cambia de colores, el reemplazo dejaría de hacer
    /// nada y los tres canales saldrían iguales sin que nadie lo notara.
    #[test]
    fn los_svg_llevan_los_colores_que_se_reemplazan() {
        for svg in [LOGO, MARCA] {
            assert!(svg.contains("#A4BBFE") && svg.contains("#7E9FFF"));
        }
    }

    #[test]
    fn el_canal_se_lee_como_lo_escribe_settings() {
        assert_eq!(Canal::de_texto("beta"), Some(Canal::Beta));
        assert_eq!(Canal::de_texto(" \"dev\""), Some(Canal::Dev));
        assert_eq!(Canal::de_texto("nightly"), None);
    }
}
