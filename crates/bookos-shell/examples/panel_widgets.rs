//! Vuelca el panel con los widgets nuevos y lo rehace en vivo con otra lista.
//!
//! `cargo run -p bookos-shell --example panel_widgets` deja en
//! `/tmp/panel_widgets/` `antes.png` y `despues.png`. Sirve para ver lo que un
//! test no ve —cómo caben el monitor, el tráfico y el consumo junto a los
//! demás— y para comprobar que `Shell::widgets_panel` cambia de verdad lo que
//! se pinta.

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder as _};

use bookos_shell::{Config, Shell};

const ANCHO: u32 = 1280;
const ESCALA: f32 = 2.0;

fn volcar(shell: &mut Shell, ruta: &std::path::Path) {
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    // B,G,R,A a R,G,B,A.
    for p in buf.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    let fichero = std::fs::File::create(ruta).expect("carpeta de salida creada");
    PngEncoder::new(std::io::BufWriter::new(fichero))
        .write_image(&buf, w, h, ExtendedColorType::Rgba8)
        .expect("PNG del tamaño del buffer");
    println!("{}  {w}×{h}", ruta.display());
}

fn lista(nombres: &[&str]) -> Vec<String> {
    nombres.iter().map(|n| (*n).to_owned()).collect()
}

fn main() {
    let salida = std::path::Path::new("/tmp/panel_widgets");
    std::fs::create_dir_all(salida).expect("crear la carpeta de salida");

    let config = Config {
        centro: None,
        derecha: lista(&[
            "escritorios",
            "monitor",
            "trafico",
            "consumo",
            "bateria",
            "red",
            "volumen",
            "reloj",
        ]),
        ..Config::default()
    };
    let mut shell = Shell::con_config(ANCHO, ESCALA, config);
    // Dos lecturas: la CPU y el tráfico son una diferencia entre dos muestras.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    shell.refresh();
    volcar(&mut shell, &salida.join("antes.png"));

    // Lo que hace Ajustes al ocultar unos, mover otros y añadir uno.
    let cambio = shell.widgets_panel(None, &lista(&["reloj", "bateria", "monitor"]));
    println!("widgets_panel devolvió {cambio}");
    shell.refresh();
    volcar(&mut shell, &salida.join("despues.png"));
    // La misma lista otra vez no debe rehacer nada.
    println!(
        "misma lista: {}",
        shell.widgets_panel(None, &lista(&["reloj", "bateria", "monitor"]))
    );
}
