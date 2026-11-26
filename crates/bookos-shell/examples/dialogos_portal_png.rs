//! Vuelca capturas 2× HiDPI de los diálogos nativos de Polkit, Access y AppChooser.
//!
//! `cargo run -p bookos-shell --example dialogos_portal_png -- <directorio>`

use std::path::Path;

use bookos_shell::tema::Tema;
use bookos_shell::{
    Config, Emergente, IdentidadPolkit, OpcionAcceso, PeticionAcceso, PeticionElegirApp,
    PeticionPolkit, Shell, TeclaPulsada,
};

const ESCALA: f32 = 2.0;
const BORDE: u32 = 48;

fn volcar(shell: &mut Shell, ruta: &Path, claro: bool) {
    let (w, h) = shell.emergente_buffer_size().expect("emergente abierta");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);

    let out_w = w + BORDE * 2;
    let out_h = h + BORDE * 2;
    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];

    for y in 0..out_h {
        let ty = y as f32 / out_h.max(1) as f32;
        for x in 0..out_w {
            let tx = x as f32 / out_w.max(1) as f32;
            let (fr, fg, fb) = if claro {
                (
                    (78.0 + 36.0 * tx + 18.0 * ty) as u8,
                    (96.0 + 32.0 * tx + 22.0 * ty) as u8,
                    (122.0 + 28.0 * tx + 24.0 * ty) as u8,
                )
            } else {
                (
                    (16.0 + 14.0 * tx + 8.0 * ty) as u8,
                    (22.0 + 18.0 * tx + 10.0 * ty) as u8,
                    (36.0 + 26.0 * tx + 14.0 * ty) as u8,
                )
            };
            let idx = ((y * out_w + x) * 4) as usize;
            rgba[idx] = fr;
            rgba[idx + 1] = fg;
            rgba[idx + 2] = fb;
            rgba[idx + 3] = 255;
        }
    }

    for y in 0..h {
        for x in 0..w {
            let s = ((y * w + x) * 4) as usize;
            let d = (((y + BORDE) * out_w + (x + BORDE)) * 4) as usize;
            let a = buf[s + 3] as u32;
            let inv = 255 - a;
            rgba[d] = (buf[s + 2] as u32 + rgba[d] as u32 * inv / 255).min(255) as u8;
            rgba[d + 1] = (buf[s + 1] as u32 + rgba[d + 1] as u32 * inv / 255).min(255) as u8;
            rgba[d + 2] = (buf[s] as u32 + rgba[d + 2] as u32 * inv / 255).min(255) as u8;
            rgba[d + 3] = 255;
        }
    }

    image::save_buffer(ruta, &rgba, out_w, out_h, image::ExtendedColorType::Rgba8)
        .expect("PNG escrito");
    println!("{} ({out_w}×{out_h})", ruta.display());
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/bookos-dialogos".to_string());
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).expect("crear carpeta de salida");

    for (sufijo, tema, claro) in [("oscuro", Tema::Oscuro, false), ("claro", Tema::Claro, true)] {
        let config = Config {
            tema,
            ..Config::default()
        };
        let mut shell = Shell::con_config(1440, ESCALA, config);
        bookos_shell::tema::aplicar(tema);
        bookos_shell::icono::refrescar_segun_tema();

        shell.abrir(Emergente::polkit(PeticionPolkit {
            id: 1,
            mensaje: "Se requiere autenticación para actualizar o instalar aplicaciones del sistema."
                .into(),
            icono: "vesktop".into(),
            detalle: Some("/usr/bin/dnf upgrade vesktop".into()),
            identidades: vec![
                IdentidadPolkit {
                    usuario: "eve".into(),
                    etiqueta: "Eve (eve)".into(),
                },
                IdentidadPolkit {
                    usuario: "root".into(),
                    etiqueta: "root".into(),
                },
            ],
        }));
        for c in "clave1234".chars() {
            shell.emergente_tecla(TeclaPulsada::Caracter(c));
        }
        shell.polkit_info(1, Some("Coloca el dedo en el lector de huellas".into()));
        volcar(&mut shell, &dir.join(format!("polkit-{sufijo}.png")), claro);

        shell.abrir(Emergente::acceso(PeticionAcceso {
            id: 2,
            app_id: "vesktop".into(),
            titulo: "Permitir acceso a la cámara".into(),
            subtitulo: "Vesktop quiere usar la cámara integrada.".into(),
            cuerpo: "Puedes revocar este permiso en cualquier momento desde Ajustes.".into(),
            icono: "camera-web-symbolic".into(),
            denegar: "Denegar".into(),
            permitir: "Permitir".into(),
            opciones: vec![
                OpcionAcceso {
                    id: "remember".into(),
                    etiqueta: "Recordar esta decisión".into(),
                    opciones: Vec::new(),
                    elegida: "true".into(),
                },
                OpcionAcceso {
                    id: "scope".into(),
                    etiqueta: "Alcance".into(),
                    opciones: vec![
                        ("once".into(), "Solo esta vez".into()),
                        ("always".into(), "Siempre".into()),
                    ],
                    elegida: "always".into(),
                },
            ],
        }));
        volcar(&mut shell, &dir.join(format!("acceso-{sufijo}.png")), claro);

        shell.abrir(Emergente::elegir_app(PeticionElegirApp {
            id: 3,
            subtitulo: "informe-2026.pdf · application/pdf".into(),
            opciones: vec![
                "org.mozilla.firefox".into(),
                "org.kde.okular".into(),
                "libreoffice-writer".into(),
                "org.gnome.Evince".into(),
                "chromium-browser".into(),
                "vesktop".into(),
            ],
            ultima: Some("org.mozilla.firefox".into()),
        }));
        volcar(
            &mut shell,
            &dir.join(format!("elegir-app-{sufijo}.png")),
            claro,
        );
    }
}
