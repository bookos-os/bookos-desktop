//! Vuelca el diálogo de color de carpeta a `/tmp/color_carpeta/`.

use bookos_shell::{Emergente, Shell};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder as _};

fn volcar(shell: &mut Shell, nombre: &str) {
    let (w, h) = shell.emergente_buffer_size().expect("diálogo abierto");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    for p in buf.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    let ruta = format!("/tmp/color_carpeta/{nombre}.png");
    let fichero = std::fs::File::create(&ruta).expect("carpeta de salida creada");
    PngEncoder::new(std::io::BufWriter::new(fichero))
        .write_image(&buf, w, h, ExtendedColorType::Rgba8)
        .expect("PNG del tamaño del buffer");
    println!("{ruta}  {w}×{h}");
}

fn main() {
    std::fs::create_dir_all("/tmp/color_carpeta").expect("crear la carpeta de salida");
    let mut shell = Shell::new(1280, 2.0);
    shell.abrir(Emergente::color_carpeta(&[("Proyectos", Some("#FF3B30"))]));
    volcar(&mut shell, "muestras");
    // El «+» es la séptima casilla: centrada en el diálogo de 310 de ancho.
    shell.emergente_pulsar(22.0 + 26.0 + 6.0 * 38.0 + 15.0, 22.0 + 168.0 + 15.0);
    volcar(&mut shell, "libre");

    bookos_shell::tema::aplicar(bookos_shell::tema::Tema::Claro);
    shell.abrir(Emergente::color_carpeta(&[(
        "Una carpeta con un nombre larguísimo que no cabe en el diálogo",
        None,
    )]));
    volcar(&mut shell, "claro");
    shell.emergente_pulsar(22.0 + 26.0 + 6.0 * 38.0 + 15.0, 22.0 + 168.0 + 15.0);
    volcar(&mut shell, "claro_libre");
}
