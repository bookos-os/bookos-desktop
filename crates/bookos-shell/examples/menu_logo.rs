//! Previsualiza el menú del logo en PNG, en los dos temas, con una fila
//! señalada y sin ninguna.
//!
//! `cargo run -p bookos-shell --example menu_logo` deja las imágenes en
//! `/tmp/menu_logo/`. El emergente sale sobre fondo transparente: el cristal
//! lo pone el compositor, así que aquí se compone sobre un degradado para ver
//! el contraste como en el escritorio.

use bookos_shell::{Config, Emergente, Shell};

const ESCALA: f32 = 2.0;

fn volcar(nombre: &str, buf: &[u8], w: u32, h: u32) {
    use image::ImageEncoder;
    // El buffer sale en B,G,R,A premultiplicado; se compone sobre un degradado
    // rosa/rojo parecido al fondo por defecto para juzgar la legibilidad.
    let mut rgba = Vec::with_capacity(buf.len());
    for (i, p) in buf.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (
            (i as u32 % w) as f32 / w as f32,
            (i as u32 / w) as f32 / h as f32,
        );
        let fondo = [
            230.0 - 20.0 * y,
            90.0 + 60.0 * (1.0 - x) * (1.0 - y),
            120.0 + 40.0 * (1.0 - y),
        ];
        let a = p[3] as f32 / 255.0;
        let c = [p[2], p[1], p[0]];
        for k in 0..3 {
            rgba.push((c[k] as f32 + fondo[k] * (1.0 - a)).round().min(255.0) as u8);
        }
        rgba.push(255);
    }
    let ruta = format!("/tmp/menu_logo/{nombre}.png");
    let f = std::fs::File::create(&ruta).expect("crear el PNG");
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .expect("escribir el PNG");
    println!("{ruta}  {w}×{h}");
}

fn main() {
    std::fs::create_dir_all("/tmp/menu_logo").expect("crear /tmp/menu_logo");
    use bookos_shell::tema::Tema::*;
    for (etiqueta, tema) in [("oscuro", Oscuro), ("claro", Claro)] {
        for (sufijo, punto) in [("", None), ("-hover", Some((60.0, 80.0)))] {
            let config = Config {
                tema,
                ..Config::default()
            };
            let mut s = Shell::con_config(1646, ESCALA, config);
            s.refresh();
            s.abrir(Emergente::menu());
            s.emergente_puntero(punto);
            // El realce entra animado: se deja terminar antes de pintar.
            std::thread::sleep(std::time::Duration::from_millis(400));
            let (w, h) = s.emergente_buffer_size().expect("menú abierto");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            s.draw_emergente(&mut buf);
            volcar(&format!("menu-{etiqueta}{sufijo}"), &buf, w, h);
        }
    }
}
