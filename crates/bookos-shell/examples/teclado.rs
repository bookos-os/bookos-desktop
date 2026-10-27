//! Vuelca el teclado en pantalla teñido por varios fondos, en claro y oscuro.
//!
//! `cargo run -p bookos-shell --example teclado` deja en `/tmp/teclado/` una
//! imagen por fondo, tema y capa. Los fondos son lisos y sintéticos: lo que se
//! mira es cómo pasa el color dominante a las teclas, y con un color conocido
//! se ve de un vistazo si el tinte se va de madre o se queda corto.

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder as _};

use bookos_shell::teclado::Capa;
use bookos_shell::tema::{self, Acento, Tema};
use bookos_shell::{Config, Shell};

const ESCALA: f32 = 2.0;
const ANCHO: u32 = 2880;

fn main() {
    let salida = std::path::Path::new("/tmp/teclado");
    if let Err(err) = std::fs::create_dir_all(salida) {
        eprintln!("no se pudo crear {}: {err}", salida.display());
        return;
    }
    let fondos = [
        ("azul", [0x2a, 0x5c, 0xc8]),
        ("naranja", [0xe0, 0x7a, 0x2c]),
        ("verde", [0x3c, 0x8c, 0x5a]),
        ("rosa", [0xd8, 0x6a, 0x9c]),
    ];
    for (nombre, rgb) in fondos {
        // 64×64 del mismo color: `paleta_del_fondo` muestrea, no le hace falta
        // un fondo de verdad para elegir el dominante.
        let rgba: Vec<u8> = std::iter::repeat_n([rgb[0], rgb[1], rgb[2], 0xff], 64 * 64)
            .flatten()
            .collect();
        tema::actualizar_acento_del_fondo(&rgba);
        for (tema_nombre, t) in [("claro", Tema::Claro), ("oscuro", Tema::Oscuro)] {
            for capa in [Capa::Letras, Capa::Simbolos] {
                let config = Config {
                    tema: t,
                    acento: Acento::Automatico,
                    ..Config::default()
                };
                let mut shell = Shell::con_config(ANCHO, ESCALA, config);
                shell.mostrar_teclado(ANCHO as f32 / ESCALA, capa);
                let Some((w, h)) = shell.teclado_buffer_size() else {
                    eprintln!("el teclado no abrió superficie");
                    return;
                };
                let mut buf = vec![0u8; (w * h * 4) as usize];
                shell.draw_teclado(&mut buf);
                for p in buf.as_chunks_mut::<4>().0 {
                    p.swap(0, 2);
                }
                let ruta =
                    salida.join(format!("{nombre}-{tema_nombre}-{capa:?}.png").to_lowercase());
                let Ok(fichero) = std::fs::File::create(&ruta) else {
                    eprintln!("no se pudo crear {}", ruta.display());
                    return;
                };
                match PngEncoder::new(std::io::BufWriter::new(fichero)).write_image(
                    &buf,
                    w,
                    h,
                    ExtendedColorType::Rgba8,
                ) {
                    Ok(()) => println!("{}  {w}×{h}", ruta.display()),
                    Err(err) => eprintln!("no se pudo escribir {}: {err}", ruta.display()),
                }
            }
        }
    }
}
