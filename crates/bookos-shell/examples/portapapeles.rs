//! Vistas del historial del portapapeles en claro, oscuro, alto contraste,
//! filtro de fotos y estado vacío, además del coste de repintado.

use bookos_shell::{
    Config, ContenidoPortapapeles, ElementoPortapapeles, Emergente, Shell, TeclaPulsada,
    miniatura_de_imagen, tema,
};
use image::{ExtendedColorType, ImageEncoder as _};

fn png_sintetico(w: u32, h: u32, r0: u8, g0: u8, b0: u8) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let fx = x as f32 / w.max(1) as f32;
            let fy = y as f32 / h.max(1) as f32;
            rgba[i] = ((r0 as f32 * (1.0 - fx)) + 230.0 * fx) as u8;
            rgba[i + 1] = ((g0 as f32 * (1.0 - fy)) + 160.0 * fy) as u8;
            rgba[i + 2] = b0;
            rgba[i + 3] = 255;
        }
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&rgba, w, h, ExtendedColorType::Rgba8)
        .expect("png válido");
    out
}

fn datos_ejemplo() -> Vec<ElementoPortapapeles> {
    let foto1 = miniatura_de_imagen(&png_sintetico(960, 540, 30, 120, 220)).expect("miniatura");
    let foto2 = miniatura_de_imagen(&png_sintetico(640, 480, 180, 60, 140)).expect("miniatura");
    vec![
        ElementoPortapapeles {
            id: 1,
            fijada: true,
            hora: "04:12".into(),
            origen: Some("Notas".into()),
            contenido: ContenidoPortapapeles::Texto(
                "cargo test -p bookos-shell -p bookos-comp -- --nocapture".into(),
            ),
        },
        ElementoPortapapeles {
            id: 2,
            fijada: false,
            hora: "04:28".into(),
            origen: Some("Captura".into()),
            contenido: ContenidoPortapapeles::Imagen {
                formato: "PNG".into(),
                miniatura: foto1,
            },
        },
        ElementoPortapapeles {
            id: 3,
            fijada: false,
            hora: "04:25".into(),
            origen: Some("Navegador".into()),
            contenido: ContenidoPortapapeles::Texto(
                "BookOS combina la claridad de One UI con el refinamiento de iOS sobre su propio compositor Wayland.".into(),
            ),
        },
        ElementoPortapapeles {
            id: 4,
            fijada: false,
            hora: "04:19".into(),
            origen: Some("Explorador".into()),
            contenido: ContenidoPortapapeles::Imagen {
                formato: "PNG".into(),
                miniatura: foto2,
            },
        },
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let salida = std::path::Path::new("/tmp/bookos-portapapeles");
    std::fs::create_dir_all(salida)?;
    let muestra = datos_ejemplo();

    for (nombre, modo, acento, alto_contraste, tabs, items) in [
        ("claro", tema::Tema::Claro, tema::Acento::Azul, false, 0, muestra.clone()),
        ("oscuro", tema::Tema::Oscuro, tema::Acento::Morado, false, 0, muestra.clone()),
        ("alto-contraste", tema::Tema::Claro, tema::Acento::Azul, true, 0, muestra.clone()),
        ("fotos", tema::Tema::Oscuro, tema::Acento::Azul, false, 2, muestra.clone()),
        ("vacio", tema::Tema::Claro, tema::Acento::Azul, false, 0, Vec::new()),
    ] {
        tema::aplicar_alto_contraste(alto_contraste);
        let mut shell = Shell::con_config(
            1280,
            2.0,
            Config {
                tema: modo,
                acento,
                ..Config::default()
            },
        );
        shell.abrir(Emergente::portapapeles(0.0, 0.0, items));
        for _ in 0..tabs {
            shell.emergente_tecla(TeclaPulsada::Tabulador);
        }
        let (w, h) = shell
            .emergente_buffer_size()
            .expect("el historial tiene superficie");
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

    tema::aplicar_alto_contraste(false);
    let mut shell = Shell::con_config(1280, 2.0, Config::default());
    shell.abrir(Emergente::portapapeles(0.0, 0.0, muestra));
    let (w, h) = shell
        .emergente_buffer_size()
        .expect("el historial tiene superficie");
    let mut buffer = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut buffer);
    let mut costes: Vec<_> = (0..30)
        .map(|_| {
            let t = std::time::Instant::now();
            shell.draw_emergente(&mut buffer);
            t.elapsed()
        })
        .collect();
    costes.sort();
    println!(
        "repintado {w}x{h}: mediana {:?}, máx {:?}",
        costes[15], costes[29]
    );
    Ok(())
}
