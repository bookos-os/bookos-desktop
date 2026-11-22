//! Los avisos con dibujo propio (Buds y batería baja), en oscuro y en claro.
//! `cargo run -p bookos-shell --example avisos_formas` → /tmp/avisos-formas/.

use bookos_shell::notificaciones::{Forma, Notificacion};
use bookos_shell::{Config, Shell, tema};

fn main() {
    let salida = std::path::Path::new(if std::env::var_os("ALTO_CONTRASTE").is_some() {
        "/tmp/avisos-formas-contraste"
    } else {
        "/tmp/avisos-formas"
    });
    std::fs::create_dir_all(salida).expect("carpeta de salida");
    let avisos = [
        (
            "buds",
            "Bluetooth",
            "Galaxy Buds3 Pro conectados",
            "El audio sale por los auriculares",
            Forma::parsear("auriculares", "Izq.=80;Dcha.=76;Estuche=18"),
            false,
        ),
        (
            "bateria",
            "Sistema",
            "Batería baja",
            "Conecta el cargador para seguir trabajando.",
            Forma::parsear("bateria", "=10"),
            true,
        ),
        (
            "normal",
            "Firefox",
            "Descarga completada",
            "informe-trimestral.pdf · 2,4 MB",
            Forma::Normal,
            false,
        ),
    ];
    for (nombre_tema, t) in [("oscuro", tema::Tema::Oscuro), ("claro", tema::Tema::Claro)] {
        tema::aplicar(t);
        for (i, (nombre, app, resumen, cuerpo, forma, critica)) in avisos.iter().enumerate() {
            let mut shell = Shell::con_config(640, 2.0, Config::default());
            // Después de crear el shell: al arrancar pone el tema de la config.
            tema::aplicar(t);
            let id = i as u32 + 1;
            let mut n = Notificacion::nueva(
                id,
                (*app).into(),
                (*resumen).into(),
                (*cuerpo).into(),
                "",
                *critica,
            );
            n.forma = forma.clone();
            shell.notificar(n, -1);
            std::thread::sleep(std::time::Duration::from_millis(400));
            let (w, h) = shell
                .toast_buffer_size(id)
                .expect("el aviso tiene superficie");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_toast(id, &mut buf);
            // Sobre el azul del fondo por defecto, del que sale el tinte.
            let rgba: Vec<u8> = buf
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let inv = 255 - u32::from(p[3]);
                    let f = [40u32, 90, 160];
                    [
                        (u32::from(p[2]) + f[0] * inv / 255) as u8,
                        (u32::from(p[1]) + f[1] * inv / 255) as u8,
                        (u32::from(p[0]) + f[2] * inv / 255) as u8,
                        255,
                    ]
                })
                .collect();
            image::save_buffer(
                salida.join(format!("{nombre}-{nombre_tema}.png")),
                &rgba,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .expect("PNG escrito");
        }
    }
    // Un mensaje con acciones, recogido y con el puntero encima.
    tema::aplicar(tema::Tema::Oscuro);
    tema::aplicar_alto_contraste(std::env::var_os("ALTO_CONTRASTE").is_some());
    let mut shell = Shell::con_config(640, 2.0, Config::default());
    tema::aplicar(tema::Tema::Oscuro);
    tema::aplicar_alto_contraste(std::env::var_os("ALTO_CONTRASTE").is_some());
    let accion = |clave: &str, etiqueta: &str| bookos_shell::notificaciones::Accion {
        clave: clave.into(),
        etiqueta: etiqueta.into(),
    };
    shell.notificar(
        Notificacion::nueva_con_datos(
            9,
            "Mensajes".into(),
            "Lucía".into(),
            "¿Quedamos el viernes para cenar?".into(),
            "",
            false,
            vec![
                accion("leido", "Marcar como leído"),
                accion("inline-reply", "Responder"),
            ],
            None,
            false,
        ),
        -1,
    );
    std::thread::sleep(std::time::Duration::from_millis(400));
    guardar(&mut shell, 9, &salida.join("mensaje-recogido.png"));
    shell.toast_señalar(9, true);
    // Los botones esperan 150 ms; el bucle real los despliega en `toasts_vivos`.
    std::thread::sleep(std::time::Duration::from_millis(270));
    shell.toasts_vivos();
    guardar(&mut shell, 9, &salida.join("mensaje-medio.png"));
    std::thread::sleep(std::time::Duration::from_millis(600));
    guardar(&mut shell, 9, &salida.join("mensaje-desplegado.png"));
    shell.toast_responder(9);
    for c in "Sí, a las nueve".chars() {
        shell.toast_tecla(9, bookos_shell::TeclaPulsada::Caracter(c));
    }
    guardar(&mut shell, 9, &salida.join("mensaje-respondiendo.png"));
    shell.confirmar_toast_respuesta(9, "Sí, a las nueve");
    guardar(&mut shell, 9, &salida.join("mensaje-enviado.png"));
    // Los Buds con sus dos botones, señalados: lo que manda bookos-budsd.
    let mut shell = Shell::con_config(640, 2.0, Config::default());
    tema::aplicar(tema::Tema::Oscuro);
    tema::aplicar_alto_contraste(std::env::var_os("ALTO_CONTRASTE").is_some());
    let mut buds = Notificacion::nueva_con_datos(
        10,
        "Bluetooth".into(),
        "Galaxy Buds3 Pro conectados".into(),
        "El audio sale por los auriculares".into(),
        "",
        false,
        vec![
            accion("sonido", "Ajustes de sonido"),
            accion("desconectar", "Desconectar"),
        ],
        None,
        false,
    );
    buds.forma = Forma::parsear("auriculares", "Izq.=80;Dcha.=76;Estuche=18");
    shell.notificar(buds, -1);
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.toast_señalar(10, true);
    std::thread::sleep(std::time::Duration::from_millis(270));
    shell.toasts_vivos();
    std::thread::sleep(std::time::Duration::from_millis(600));
    guardar(&mut shell, 10, &salida.join("buds-desplegado.png"));
    shell.confirmar_toast_boton(10, "desconectar");
    guardar(&mut shell, 10, &salida.join("buds-hecho.png"));
    println!("guardados en {}", salida.display());
}

fn guardar(shell: &mut Shell, id: u32, ruta: &std::path::Path) {
    let (w, h) = shell
        .toast_buffer_size(id)
        .expect("el aviso tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_toast(id, &mut buf);
    let rgba: Vec<u8> = buf
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let inv = 255 - u32::from(p[3]);
            [
                (u32::from(p[2]) + 40 * inv / 255) as u8,
                (u32::from(p[1]) + 90 * inv / 255) as u8,
                (u32::from(p[0]) + 160 * inv / 255) as u8,
                255,
            ]
        })
        .collect();
    image::save_buffer(ruta, &rgba, w, h, image::ExtendedColorType::Rgba8).expect("PNG escrito");
}
