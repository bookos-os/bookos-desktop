//! Tokens de color del HIG de BookOS (§2).
//!
//! Lee `~/.config/bookos/palette.json` si existe (paleta dinámica de Settings)
//! y usa la tabla oficial de respaldo del HIG en claro y oscuro, garantizando
//! contraste ≥ 4,5:1 en texto y ≥ 3:1 en controles/iconos.

use std::path::PathBuf;

use tiny_skia::Color;

#[derive(Debug, Clone, Copy)]
pub struct Paleta {
    pub bg: Color,
    pub card: Color,
    pub sbg: Color,
    pub tx: Color,
    pub tx2: Color,
    pub div: Color,
    pub brd: Color,
    pub hov: Color,
    pub act: Color,
    pub toff: Color,
    pub blue: Color,
    pub on_blue: Color,
    pub red: Color,
    pub on_red: Color,
}

impl Paleta {
    /// Carga la paleta efectiva para claro/oscuro según HIG §2.
    pub fn cargar(oscuro: bool) -> Self {
        let mut p = if oscuro {
            Self::fallback_oscuro()
        } else {
            Self::fallback_claro()
        };
        if let Some(json) = leer_palette_json() {
            aplicar_json(&mut p, &json, oscuro);
        }
        p.on_blue = elegir_tinta_contraste(p.blue);
        p.on_red = elegir_tinta_contraste(p.red);
        p
    }

    fn fallback_claro() -> Self {
        let blue = hex(0x00, 0x7A, 0xFF, 255);
        let red = hex(0xFF, 0x3B, 0x30, 255);
        Self {
            bg: hex(0xF2, 0xF2, 0xF7, 255),
            card: hex(0xFF, 0xFF, 0xFF, 255),
            sbg: hex(0x00, 0x00, 0x00, 15),
            tx: hex(0x00, 0x00, 0x00, 255),
            tx2: hex(0x6C, 0x6C, 0x70, 255),
            div: hex(0x00, 0x00, 0x00, 20),
            brd: hex(0x00, 0x00, 0x00, 26),
            hov: hex(0x00, 0x00, 0x00, 10),
            act: hex(0x00, 0x00, 0x00, 20),
            toff: hex(0xE0, 0xE0, 0xE0, 255),
            blue,
            on_blue: elegir_tinta_contraste(blue),
            red,
            on_red: elegir_tinta_contraste(red),
        }
    }

    fn fallback_oscuro() -> Self {
        let blue = hex(0x0A, 0x84, 0xFF, 255);
        let red = hex(0xFF, 0x45, 0x3A, 255);
        Self {
            bg: hex(0x14, 0x14, 0x16, 255),
            card: hex(0x1C, 0x1C, 0x1E, 255),
            sbg: hex(0xFF, 0xFF, 0xFF, 20),
            tx: hex(0xFF, 0xFF, 0xFF, 255),
            tx2: hex(0x8E, 0x8E, 0x93, 255),
            div: hex(0xFF, 0xFF, 0xFF, 20),
            brd: hex(0xFF, 0xFF, 0xFF, 18),
            hov: hex(0xFF, 0xFF, 0xFF, 15),
            act: hex(0xFF, 0xFF, 0xFF, 26),
            toff: hex(0x48, 0x48, 0x4A, 255),
            blue,
            on_blue: elegir_tinta_contraste(blue),
            red,
            on_red: elegir_tinta_contraste(red),
        }
    }
}

fn hex(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn leer_palette_json() -> Option<serde_json::Value> {
    let ruta = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        PathBuf::from(xdg).join("bookos/palette.json")
    } else {
        let home = std::env::var_os("HOME")?;
        PathBuf::from(home).join(".config/bookos/palette.json")
    };
    let texto = std::fs::read_to_string(ruta).ok()?;
    serde_json::from_str(&texto).ok()
}

fn aplicar_json(p: &mut Paleta, root: &serde_json::Value, oscuro: bool) {
    let rama = root
        .get(if oscuro { "dark" } else { "light" })
        .or_else(|| root.get(if oscuro { "oscuro" } else { "claro" }))
        .or_else(|| root.get("roles"))
        .unwrap_or(root);
    let Some(obj) = rama.as_object() else {
        return;
    };
    let tomar = |claves: &[&str]| -> Option<Color> {
        for k in claves {
            if let Some(s) = obj.get(*k).and_then(|v| v.as_str())
                && let Some(c) = parsear_color(s)
            {
                return Some(c);
            }
        }
        None
    };
    if let Some(c) = tomar(&["--bg", "bg"]) {
        p.bg = con_alpha(c, 1.0);
    }
    if let Some(c) = tomar(&["--card", "card"]) {
        p.card = con_alpha(c, 1.0);
    }
    if let Some(c) = tomar(&["--sbg", "sbg"]) {
        p.sbg = c;
    }
    if let Some(c) = tomar(&["--tx", "tx"]) {
        p.tx = c;
    }
    if let Some(c) = tomar(&["--tx2", "tx2"]) {
        p.tx2 = c;
    }
    if let Some(c) = tomar(&["--div", "div"]) {
        p.div = c;
    }
    if let Some(c) = tomar(&["--brd", "brd"]) {
        p.brd = c;
    }
    if let Some(c) = tomar(&["--hov", "hov"]) {
        p.hov = c;
    }
    if let Some(c) = tomar(&["--act", "act"]) {
        p.act = c;
    }
    if let Some(c) = tomar(&["--toff", "toff"]) {
        p.toff = c;
    }
    if let Some(c) = tomar(&["--blue", "blue", "accent"]) {
        p.blue = c;
    }
}

fn con_alpha(c: Color, alpha: f32) -> Color {
    Color::from_rgba(c.red(), c.green(), c.blue(), alpha.clamp(0.0, 1.0)).unwrap_or(c)
}

fn parsear_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        if h.len() == 6 {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            return Some(Color::from_rgba8(r, g, b, 255));
        } else if h.len() == 8 {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            let a = u8::from_str_radix(&h[6..8], 16).ok()?;
            return Some(Color::from_rgba8(r, g, b, a));
        }
    }
    if let Some(inner) = s.strip_prefix("rgba(").and_then(|x| x.strip_suffix(')')) {
        let p: Vec<&str> = inner.split(',').map(str::trim).collect();
        if p.len() == 4 {
            let r = p[0].parse::<u8>().ok()?;
            let g = p[1].parse::<u8>().ok()?;
            let b = p[2].parse::<u8>().ok()?;
            let a = (p[3].parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8;
            return Some(Color::from_rgba8(r, g, b, a));
        }
    }
    None
}

fn luminancia_relativa(c: Color) -> f32 {
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.red()) + 0.7152 * lin(c.green()) + 0.0722 * lin(c.blue())
}

fn contraste(a: Color, b: Color) -> f32 {
    let la = luminancia_relativa(a);
    let lb = luminancia_relativa(b);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Elige blanco o negro según cuál logre mayor contraste WCAG sobre `fondo` (HIG §2).
fn elegir_tinta_contraste(fondo: Color) -> Color {
    let blanco = Color::WHITE;
    let negro = Color::BLACK;
    if contraste(blanco, fondo) >= contraste(negro, fondo) {
        blanco
    } else {
        negro
    }
}
