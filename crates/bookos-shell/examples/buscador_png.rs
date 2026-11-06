//! El buscador a mitad de sus animaciones, en PNG, y lo que cuesta pintarlo.
//!
//! `cargo run -p bookos-shell --example buscador_png` → `/tmp/buscador/`.

use bookos_shell::{Config, Emergente, Shell, TeclaPulsada};
use std::time::{Duration, Instant};

const ESCALA: f32 = 2.0;

fn guardar(shell: &mut Shell, nombre: &str) -> Duration {
    let (w, h) = shell.emergente_buffer_size().expect("buscador abierto");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let t = Instant::now();
    shell.draw_emergente(&mut buf);
    let coste = t.elapsed();
    let rgba: Vec<u8> = buf
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let inv = 255 - p[3] as u32;
            let f = [40u32, 90, 160];
            [
                (p[2] as u32 + f[0] * inv / 255) as u8,
                (p[1] as u32 + f[1] * inv / 255) as u8,
                (p[0] as u32 + f[2] * inv / 255) as u8,
                255,
            ]
        })
        .collect();
    image::save_buffer(
        format!("/tmp/buscador/{nombre}.png"),
        &rgba,
        w,
        h,
        image::ExtendedColorType::Rgba8,
    )
    .expect("PNG escrito");
    coste
}

fn main() {
    std::fs::create_dir_all("/tmp/buscador").expect("carpeta de salida");
    let mut shell = Shell::con_config(1440, ESCALA, Config::default());
    shell.abrir(Emergente::buscador_en((1440.0, 900.0), Vec::new()));
    println!("abierto: {:?}", guardar(&mut shell, "0-abierto"));
    for c in "zzzz".chars() {
        shell.emergente_tecla(TeclaPulsada::Caracter(c));
    }
    std::thread::sleep(Duration::from_millis(400));
    guardar(&mut shell, "1-vacio");
    shell.emergente_tecla(TeclaPulsada::BorrarTodo);
    std::thread::sleep(Duration::from_millis(60));
    println!("creciendo: {:?}", guardar(&mut shell, "2-creciendo"));
    std::thread::sleep(Duration::from_millis(400));
    guardar(&mut shell, "3-crecido");
    shell.emergente_tecla(TeclaPulsada::Abajo);
    shell.emergente_tecla(TeclaPulsada::Abajo);
    std::thread::sleep(Duration::from_millis(50));
    println!("deslizando: {:?}", guardar(&mut shell, "4-deslizando"));
    std::thread::sleep(Duration::from_millis(400));
    guardar(&mut shell, "5-elegida");
    let mut costes: Vec<Duration> = (0..30)
        .map(|_| {
            let (w, h) = shell.emergente_buffer_size().expect("abierto");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let t = Instant::now();
            shell.draw_emergente(&mut buf);
            t.elapsed()
        })
        .collect();
    costes.sort();
    println!("repintado: mediana {:?}, máx {:?}", costes[15], costes[29]);
    // Acciones y datos con sus insignias junto a las aplicaciones.
    for (nombre, consulta) in [
        ("6-acciones", "modo"),
        ("7-datos", "bat"),
        ("8-ajuste", "brillo 30"),
    ] {
        shell.emergente_tecla(TeclaPulsada::BorrarTodo);
        for c in consulta.chars() {
            shell.emergente_tecla(TeclaPulsada::Caracter(c));
        }
        std::thread::sleep(Duration::from_millis(400));
        guardar(&mut shell, nombre);
    }
}
