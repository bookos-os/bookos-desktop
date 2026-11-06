//! Previsualiza la pantalla de bloqueo en PNG, sin compositor.
//!
//! `cargo run -p bookos-shell --example bloqueo [carátula]` deja en
//! `/tmp/bloqueo/` una imagen por tema, con y sin música. El buffer del bloqueo
//! es transparente; aquí se compone sobre el fondo de `panel.conf` y el velo
//! de la canción, que en la sesión pone el compositor, para juzgar la tarjeta
//! contra lo que de verdad tiene detrás.

use bookos_shell::tema::Tema;
use bookos_shell::{Config, Shell};

const ESCALA: f32 = 1.75;

fn main() {
    std::fs::create_dir_all("/tmp/bloqueo").expect("crear /tmp/bloqueo");
    let caratula = std::env::args().nth(1);
    let base = Config::cargar();
    for (etiqueta, tema, fondo) in [
        ("oscuro", Tema::Oscuro, base.fondo_oscuro.clone()),
        ("claro", Tema::Claro, base.fondo_claro.clone()),
    ] {
        for (variante, sonando) in [
            ("sin-musica", None),
            ("menu", None),
            ("musica", Some(cancion(None))),
            ("caratula", caratula.as_deref().map(|c| cancion(Some(c)))),
        ] {
            if variante == "caratula" && sonando.is_none() {
                continue;
            }
            let mut config = Config {
                tema,
                ..Config::cargar()
            };
            // Con animaciones, la foto saldría a mitad de la entrada.
            config.bloqueo.animaciones = false;
            let pantalla = (2880.0 / ESCALA, 1800.0 / ESCALA);
            let mut shell = Shell::con_config(pantalla.0 as u32, ESCALA, config);
            shell.bloquear("10:42".into(), "lunes, 29 de septiembre".into(), pantalla);
            shell.bloqueo_medio(sonando);
            // añadido en la 0.2.0 -- el menú de apagado desplegado, que es donde se ven sus iconos.
            if variante == "menu" {
                let b = shell.bloqueo_mut().expect("bloqueo puesto");
                let (x, y, lado, _) = b.boton_energia(pantalla);
                shell.bloqueo_pulsado(x + lado / 2.0, y + lado / 2.0);
                // El menú se abre con muelle aunque las animaciones del bloqueo
                // estén apagadas: se espera a que asiente.
                std::thread::sleep(std::time::Duration::from_millis(600));
            }
            let (w, h) = shell.bloqueo_buffer_size().expect("bloqueo puesto");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_bloqueo(&mut buf);
            // añadido en la 0.2.0 -- los PNG de cada capa permiten detectar recortes sin arrancar una sesión.
            for (capa, (_, _, cw, ch)) in shell.bloqueo_capas() {
                let cw = (bookos_shell::a_pixel_entero(cw, ESCALA) * ESCALA) as u32;
                let ch = (bookos_shell::a_pixel_entero(ch, ESCALA) * ESCALA) as u32;
                let mut capa_buf = vec![0u8; (cw * ch * 4) as usize];
                shell.draw_bloqueo_capa(capa, &mut capa_buf);
                volcar(
                    &format!("{variante}-{etiqueta}-{capa:?}"),
                    &capa_buf,
                    cw,
                    ch,
                    None,
                    None,
                );
            }
            volcar(
                &format!("{variante}-{etiqueta}"),
                &buf,
                w,
                h,
                fondo.as_deref(),
                shell.bloqueo_velo(),
            );
        }
    }
}

fn cancion(caratula: Option<&str>) -> bookos_shell::medios::Sonando {
    bookos_shell::medios::Sonando {
        bus: "org.mpris.MediaPlayer2.prueba".into(),
        aplicacion: "Prueba".into(),
        caratula: caratula.map(str::to_owned),
        titulo: "Cielo de invierno".into(),
        artista: "La Habitación Roja".into(),
        reproduciendo: true,
        posicion: Some(78),
        duracion: Some(214),
    }
}

fn volcar(
    nombre: &str,
    buf: &[u8],
    w: u32,
    h: u32,
    fondo: Option<&str>,
    velo: Option<iced_core::Color>,
) {
    let mut lienzo = fondo
        .and_then(|f| image::open(f).ok())
        .map(|i| {
            i.resize_to_fill(w, h, image::imageops::FilterType::Triangle)
                .to_rgba8()
        })
        .unwrap_or_else(|| image::RgbaImage::from_pixel(w, h, image::Rgba([40, 40, 48, 255])));
    // El velo baja desde arriba y se apaga al 60 % del alto, como el del
    // compositor.
    if let Some(v) = velo {
        for (_, y, p) in lienzo.enumerate_pixels_mut() {
            let a = v.a * (1.0 - y as f32 / (h as f32 * 0.6)).max(0.0);
            for (c, o) in p.0.iter_mut().zip([v.r, v.g, v.b]) {
                *c = (*c as f32 * (1.0 - a) + o * 255.0 * a) as u8;
            }
        }
    }
    // El buffer viene en B,G,R,A premultiplicado.
    for (p, q) in lienzo.pixels_mut().zip(buf.as_chunks::<4>().0) {
        let a = q[3] as f32 / 255.0;
        for (c, o) in p.0.iter_mut().zip([q[2], q[1], q[0]]) {
            *c = (o as f32 + *c as f32 * (1.0 - a)).min(255.0) as u8;
        }
    }
    let ruta = format!("/tmp/bloqueo/{nombre}.png");
    lienzo.save(&ruta).expect("escribir PNG");
    println!("{ruta}  {w}×{h}");
}
