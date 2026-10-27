//! Vistas del selector en ambos temas y acentos, sin abrir una sesión.

use bookos_shell::{Config, Emergente, Shell, TeclaPulsada, tema};
use image::{ExtendedColorType, ImageEncoder as _};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let salida = std::path::Path::new("/tmp/bookos-emojis");
    std::fs::create_dir_all(salida)?;
    for (nombre, modo, acento, consulta, recientes) in [
        ("claro", tema::Tema::Claro, tema::Acento::Azul, "", vec![]),
        (
            "oscuro",
            tema::Tema::Oscuro,
            tema::Acento::Morado,
            "",
            vec![],
        ),
        (
            "busqueda",
            tema::Tema::Claro,
            tema::Acento::Naranja,
            "corazon",
            vec![],
        ),
        (
            "recientes",
            tema::Tema::Oscuro,
            tema::Acento::Verde,
            "",
            vec![
                "🧑‍💻".into(),
                "🇪🇸".into(),
                "❤️".into(),
                "👏".into(),
                "🎉".into(),
                "☕".into(),
            ],
        ),
        (
            "vacio",
            tema::Tema::Claro,
            tema::Acento::Azul,
            "zzzzzz",
            vec![],
        ),
    ] {
        let mut shell = Shell::con_config(
            1280,
            2.0,
            Config {
                tema: modo,
                acento,
                ..Config::default()
            },
        );
        shell.abrir(Emergente::emojis(0.0, 0.0, &recientes));
        for c in consulta.chars() {
            shell.emergente_tecla(TeclaPulsada::Caracter(c));
        }
        let (w, h) = shell
            .emergente_buffer_size()
            .expect("el selector tiene superficie");
        let mut buffer = vec![0; (w * h * 4) as usize];
        shell.draw_emergente(&mut buffer);
        for pixel in buffer.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let ruta = salida.join(format!("{nombre}.png"));
        image::codecs::png::PngEncoder::new(std::fs::File::create(&ruta)?).write_image(
            &buffer,
            w,
            h,
            ExtendedColorType::Rgba8,
        )?;
        println!("{}", ruta.display());
    }
    Ok(())
}
