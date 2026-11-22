//! Previsualiza en PNG los menús contextuales del escritorio y del dock y sus
//! diálogos (pegar, renombrar, información, vista rápida), en los dos temas.
//!
//! `cargo run -p bookos-shell --example menu_contextual` deja las imágenes en
//! `/tmp/menu_contextual/`. El escritorio es uno de mentira en
//! `/tmp/menu_contextual/Escritorio` (vía `BOOKOS_ESCRITORIO`): no toca el de
//! verdad. Lo emergente se compone sobre un degradado oscuro para juzgar el
//! contraste como sobre un fondo de pantalla.

use bookos_shell::{Config, Shell};

const ESCALA: f32 = 2.0;
const DIR: &str = "/tmp/menu_contextual";

fn volcar(nombre: &str, buf: &[u8], w: u32, h: u32) {
    use image::ImageEncoder;
    let mut rgba = Vec::with_capacity(buf.len());
    for (i, p) in buf.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (
            (i as u32 % w) as f32 / w as f32,
            (i as u32 / w) as f32 / h as f32,
        );
        let fondo = [40.0 + 30.0 * x, 60.0 + 20.0 * y, 110.0 - 30.0 * y];
        let a = p[3] as f32 / 255.0;
        let c = [p[2], p[1], p[0]];
        for k in 0..3 {
            rgba.push((c[k] as f32 + fondo[k] * (1.0 - a)).round().min(255.0) as u8);
        }
        rgba.push(255);
    }
    let ruta = format!("{DIR}/{nombre}.png");
    let f = std::fs::File::create(&ruta).expect("crear el PNG");
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .expect("escribir el PNG");
    println!("{ruta}  {w}×{h}");
}

fn pintar(s: &mut Shell, nombre: &str) {
    // El realce entra animado: se deja terminar antes de pintar.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (w, h) = s.emergente_buffer_size().expect("emergente abierta");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    s.draw_emergente(&mut buf);
    volcar(nombre, &buf, w, h);
}

/// Un escritorio con carpetas, un PDF, un script, una imagen y un acceso
/// directo, para que cada menú tenga lo suyo.
fn preparar() -> std::path::PathBuf {
    let esc = std::path::PathBuf::from(DIR).join("Escritorio");
    let _ = std::fs::remove_dir_all(&esc);
    std::fs::create_dir_all(esc.join("Proyectos/a")).expect("carpeta");
    std::fs::create_dir_all(esc.join("Fotos 2026")).expect("carpeta");
    std::fs::write(esc.join("Informe.pdf"), vec![0u8; 2_400_000]).expect("pdf");
    std::fs::write(
        esc.join("copia.sh"),
        "#!/bin/sh\n# Copia de seguridad\nrsync -a ~/Documentos /mnt/copia\necho hecho\n",
    )
    .expect("script");
    let img = image::RgbaImage::from_fn(640, 400, |x, y| {
        image::Rgba([(x / 3) as u8, (y / 2) as u8, 180, 255])
    });
    img.save(esc.join("Atardecer.png")).expect("png");
    let original = std::path::PathBuf::from(DIR).join("Documentos");
    std::fs::create_dir_all(&original).expect("original");
    std::os::unix::fs::symlink(&original, esc.join("Documentos")).expect("enlace");
    esc
}

fn seleccionar(s: &mut Shell, nombres: &[&str]) {
    let e = s.escritorio_mut();
    e.seleccionar(None, false);
    for n in 0..e.cuantos() {
        e.seleccionar(Some(n), true);
        let nombre = e.seleccionados().last().map(|x| x.nombre.clone());
        if !nombres.iter().any(|q| Some(q.to_string()) == nombre) {
            e.seleccionar(Some(n), true);
        }
    }
}

fn main() {
    std::fs::create_dir_all(DIR).expect("crear la carpeta");
    let esc = preparar();
    // SAFETY: un solo hilo y antes de crear el shell, que es quien lo lee.
    unsafe { std::env::set_var("BOOKOS_ESCRITORIO", &esc) };
    use bookos_shell::tema::Tema::*;
    for (etiqueta, tema) in [("oscuro", Oscuro), ("claro", Claro)] {
        let config = Config {
            tema,
            ..Config::default()
        };
        let mut s = Shell::con_config(1646, ESCALA, config);
        s.refresh();
        s.escritorio_pantalla((1440.0, 900.0), ESCALA);

        // Fondo vacío, con «Ordenar por ›» desplegado.
        seleccionar(&mut s, &[]);
        s.menu_escritorio((100.0, 100.0), true);
        s.emergente_puntero(Some((40.0, 5.0 + 30.0 * 3.0 + 11.0 + 15.0)));
        pintar(&mut s, &format!("fondo-{etiqueta}"));

        for (caso, nombres) in [
            ("carpeta", &["Proyectos"][..]),
            ("pdf", &["Informe.pdf"]),
            ("script", &["copia.sh"]),
            ("acceso", &["Documentos"]),
            ("varios", &["Proyectos", "Fotos 2026", "Informe.pdf"]),
        ] {
            seleccionar(&mut s, nombres);
            s.menu_escritorio((100.0, 100.0), false);
            // Sobre el segundo botón rápido, para ver su realce.
            s.emergente_puntero(Some((140.0, 5.0 + 50.0 + 20.0)));
            pintar(&mut s, &format!("{caso}-{etiqueta}"));
        }

        // Dock: tres ventanas, una, y cerrada.
        for (caso, ventanas) in [
            (
                "dock-varias",
                vec![
                    (
                        "org.kde.dolphin".to_string(),
                        "Documentación de Smithay".to_string(),
                        true,
                    ),
                    (
                        "org.kde.dolphin".to_string(),
                        "Bandeja de entrada".to_string(),
                        false,
                    ),
                    (
                        "org.kde.dolphin".to_string(),
                        "Diseño de BookUI".to_string(),
                        false,
                    ),
                ],
            ),
            (
                "dock-una",
                vec![("org.kde.dolphin".into(), "Descargas".into(), true)],
            ),
            ("dock-cerrada", Vec::new()),
        ] {
            let centro_y = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
            let x = bookos_shell::DOCK_PAD
                + bookos_shell::DOCK_ICON / 2.0
                + 2.0 * (bookos_shell::DOCK_ICON + 14.0);
            // Las ventanas, de la app que haya en ese icono.
            let app_id = s.dock_item_en(x, centro_y).map(|a| a.0).unwrap_or_default();
            let ventanas: Vec<(String, String, bool)> = ventanas
                .into_iter()
                .map(|(_, t, a)| (app_id.clone(), t, a))
                .collect();
            s.dock_ventanas(&[app_id.clone()]);
            if s.dock_menu(x, centro_y, &ventanas) {
                pintar(&mut s, &format!("{caso}-{etiqueta}"));
            } else {
                println!("{caso}: el icono {x} no tiene menú");
            }
        }

        // Diálogos.
        s.abrir_pegar(&[std::path::PathBuf::from("/usr/share/backgrounds")]);
        pintar(&mut s, &format!("pegar-{etiqueta}"));
        seleccionar(&mut s, &["Informe.pdf"]);
        s.abrir_renombrar();
        pintar(&mut s, &format!("renombrar-{etiqueta}"));
        s.renombrar_a("Fotos 2026");
        pintar(&mut s, &format!("renombrar-error-{etiqueta}"));
        seleccionar(&mut s, &["copia.sh"]);
        s.abrir_informacion();
        pintar(&mut s, &format!("info-script-{etiqueta}"));
        seleccionar(&mut s, &["Proyectos"]);
        s.abrir_informacion();
        pintar(&mut s, &format!("info-carpeta-{etiqueta}"));
        seleccionar(&mut s, &["Atardecer.png"]);
        s.abrir_vista_rapida();
        pintar(&mut s, &format!("vista-{etiqueta}"));
        seleccionar(&mut s, &["copia.sh"]);
        s.abrir_vista_rapida();
        pintar(&mut s, &format!("vista-texto-{etiqueta}"));
    }
}
