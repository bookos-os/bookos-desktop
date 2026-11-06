//! Previsualiza la isla de actividades en PNG, sin compositor.
//!
//! `cargo run -p bookos-shell --example actividad` deja en `/tmp/actividad/`
//! una imagen por variante y por tema. Existe porque el diseño de la isla no se
//! puede juzgar leyendo el árbol de widgets: hay que **verla**, y arrancar la
//! sesión entera para mirar una tarjeta es el camino largo.
//!
//! El fondo del PNG es transparente, igual que el buffer real: la isla cuelga
//! del borde superior de la pantalla y su sombra tiene que verse contra lo que
//! haya debajo.

use bookos_shell::actividad::{Clase, Estado, Fase, ItemCola, Metrica, Portada};
use bookos_shell::{Config, Shell};

const ESCALA: f32 = 2.0;

fn base(clase: Clase) -> Estado {
    Estado {
        app_id: match clase {
            Clase::Player => "com.bookos.player",
            Clase::Timer => "com.bookos.clock",
            Clase::Recorder => "com.bookos.voicerecorder",
            Clase::Transfer => "com.bookos.settings",
            Clase::Calendar => "com.bookos.calendar",
            Clase::Monitor => "com.bookos.systemmonitor",
            Clase::Auriculares => "com.bookos.buds",
            Clase::Asistente(_) => "com.bookos.assistant",
        }
        .into(),
        clase,
        activo: true,
        pausado: false,
        titulo: "Cielo de invierno".into(),
        subtitulo: "La Habitación Roja".into(),
        posicion_ms: 78_000,
        duracion_ms: 214_000,
        restante_ms: 154_000,
        volumen: 65,
        nivel: 0.7,
        aleatorio: true,
        repetir: false,
        portada: Some(portada()),
        metricas: Vec::new(),
        cola: vec![
            item("1", "Cielo de invierno", "La Habitación Roja", true, true),
            item("2", "Ayer", "Los Planetas", false, false),
            item("3", "Mediterráneo", "Serrat", true, false),
            item(
                "4",
                "La estatua del jardín botánico",
                "Radio Futura",
                false,
                false,
            ),
        ],
    }
}

fn transferencia() -> Estado {
    Estado {
        titulo: "Apuntes_Tema3.pdf".into(),
        subtitulo: "Enviando a S22 Ultra · 8,4 MB/s".into(),
        nivel: 0.42,
        restante_ms: 72_000,
        portada: None,
        cola: Vec::new(),
        ..base(Clase::Transfer)
    }
}

fn clase() -> Estado {
    Estado {
        titulo: "Programación de sistemas".into(),
        subtitulo: "10:00–11:30 · Aula 3".into(),
        restante_ms: 9 * 60_000,
        portada: None,
        cola: Vec::new(),
        ..base(Clase::Calendar)
    }
}

fn monitor() -> Estado {
    let m = |e: &str, t: &str, critica| Metrica {
        etiqueta: e.into(),
        valor: 0.0,
        texto: t.into(),
        critica,
    };
    Estado {
        titulo: "Rendimiento".into(),
        metricas: vec![
            m("CPU", "23 %", false),
            m("GPU", "12 %", false),
            m("RAM", "41 %", false),
            m("Temperatura", "62°", false),
            m("Batería", "12 %", true),
        ],
        portada: None,
        cola: Vec::new(),
        ..base(Clase::Monitor)
    }
}

/// Lo que publica bookos-budsd al sacar los Buds, con el derecho bajo.
fn auriculares() -> Estado {
    let m = |e: &str, v: u8| Metrica {
        etiqueta: e.into(),
        valor: f32::from(v) / 100.0,
        texto: format!("{v} %"),
        critica: v <= 20,
    };
    Estado {
        titulo: "Buds3 Pro de Evelyn".into(),
        metricas: vec![m("Izquierdo", 59), m("Derecho", 18), m("Estuche", 40)],
        portada: None,
        cola: Vec::new(),
        ..base(Clase::Auriculares)
    }
}

// añadido en la 0.2.0 -- las fases de Zenith, con lo que publicaría en cada una.
fn asistente(fase: Fase, titulo: &str, subtitulo: &str) -> Estado {
    Estado {
        titulo: titulo.into(),
        subtitulo: subtitulo.into(),
        portada: None,
        cola: Vec::new(),
        ..base(Clase::Asistente(fase))
    }
}

fn item(id: &str, titulo: &str, artista: &str, favorita: bool, actual: bool) -> ItemCola {
    ItemCola {
        id: id.into(),
        titulo: titulo.into(),
        artista: artista.into(),
        duracion_ms: 234_000,
        favorita,
        actual,
        // Una de cada dos con carátula, para ver las dos filas lado a lado.
        portada: (id.len() == 1 && id != "1" && id.as_bytes()[0] % 2 == 1).then(portada),
    }
}

/// Una portada inventada: un degradado diagonal, que es lo que hace falta para
/// ver si el recorte y la sombra del arte están bien.
fn portada() -> Portada {
    let (w, h) = (300u32, 300u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let t = (x + y) as f32 / (w + h) as f32;
            rgba.extend_from_slice(&[
                (30.0 + 200.0 * t) as u8,
                (90.0 + 60.0 * (1.0 - t)) as u8,
                (200.0 - 60.0 * t) as u8,
                255,
            ]);
        }
    }
    Portada {
        rgba,
        width: w,
        height: h,
    }
}

fn volcar(nombre: &str, buf: &[u8], w: u32, h: u32) {
    use image::ImageEncoder;
    // El buffer sale en B,G,R,A y el PNG se escribe en R,G,B,A.
    let mut rgba = buf.to_vec();
    for p in rgba.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    let ruta = format!("/tmp/actividad/{nombre}.png");
    let f = std::fs::File::create(&ruta).unwrap();
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    println!("{ruta}  {w}×{h}");
}

/// Pinta una variante. `clics` son pulsaciones lógicas que se dan antes de
/// dibujar, para llegar a la vista abierta o a la cola por el mismo camino que
/// las daría el usuario.
fn pinta(nombre: &str, tema: bookos_shell::tema::Tema, estado: Estado, clics: &[(f32, f32)]) {
    pinta_varias(nombre, tema, vec![estado], clics);
}

/// Varias actividades vivas a la vez: la principal en la píldora y las demás
/// como burbujas.
fn pinta_varias(
    nombre: &str,
    tema: bookos_shell::tema::Tema,
    estados: Vec<Estado>,
    clics: &[(f32, f32)],
) {
    pinta_varias_a_escala(nombre, tema, estados, clics, ESCALA);
}

fn pinta_varias_a_escala(
    nombre: &str,
    tema: bookos_shell::tema::Tema,
    estados: Vec<Estado>,
    clics: &[(f32, f32)],
    escala: f32,
) {
    // El temporizador solo sale si la configuración lo deja verse siempre; sin
    // esto la vista del temporizador en marcha no se puede previsualizar.
    let actividades = bookos_shell::ConfigActividades {
        temporizador_siempre: true,
        ..Default::default()
    };
    let config = Config {
        tema,
        actividades,
        ..Config::default()
    };
    let mut shell = Shell::con_config(1646, escala, config);
    for estado in estados {
        shell.publicar_actividad(estado);
    }
    for (x, y) in clics {
        shell.actividad_pulsar(*x, *y);
    }
    let Some((w, h)) = shell.actividad_buffer_size() else {
        println!("{nombre}: la isla no se publicó");
        return;
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_actividad(&mut buf);
    volcar(nombre, &buf, w, h);
}

fn main() {
    std::fs::create_dir_all("/tmp/actividad").unwrap();
    use bookos_shell::tema::Tema::*;

    // El primer clic cae en el centro de la tarjeta compacta y la abre; el
    // segundo, en la esquina inferior izquierda del reproductor, saca la cola.
    for (etiqueta, tema) in [("oscuro", Oscuro), ("claro", Claro)] {
        pinta(
            &format!("player-compacto-{etiqueta}"),
            tema,
            base(Clase::Player),
            &[],
        );
        pinta(
            &format!("player-abierto-{etiqueta}"),
            tema,
            base(Clase::Player),
            &[(180.0, 55.0)],
        );
        pinta(
            &format!("player-cola-{etiqueta}"),
            tema,
            base(Clase::Player),
            // El segundo clic cae en «A continuación», en la fila de debajo
            // del progreso: 26 de margen + la posición que le dan las
            // constantes.
            &[(180.0, 55.0), (26.0 + 60.0, 26.0 + 235.0)],
        );
        pinta(
            &format!("timer-compacto-{etiqueta}"),
            tema,
            base(Clase::Timer),
            &[],
        );
        pinta(
            &format!("timer-abierto-{etiqueta}"),
            tema,
            base(Clase::Timer),
            &[(180.0, 55.0)],
        );
        let mut vencido = base(Clase::Timer);
        vencido.restante_ms = -7_000;
        pinta(
            &format!("timer-vencido-{etiqueta}"),
            tema,
            vencido,
            &[(180.0, 55.0)],
        );
        pinta(
            &format!("grabadora-compacta-{etiqueta}"),
            tema,
            base(Clase::Recorder),
            &[],
        );
        pinta(
            &format!("grabadora-abierta-{etiqueta}"),
            tema,
            base(Clase::Recorder),
            &[(180.0, 55.0)],
        );
        pinta_varias(
            &format!("tres-a-la-vez-{etiqueta}"),
            tema,
            vec![
                base(Clase::Player),
                base(Clase::Timer),
                base(Clase::Recorder),
            ],
            &[],
        );
        for (nombre, estado) in [
            ("transferencia", transferencia()),
            ("clase", clase()),
            ("monitor", monitor()),
            ("auriculares", auriculares()),
        ] {
            pinta(
                &format!("{nombre}-compacto-{etiqueta}"),
                tema,
                estado.clone(),
                &[],
            );
            pinta(
                &format!("{nombre}-abierto-{etiqueta}"),
                tema,
                estado,
                &[(180.0, 55.0)],
            );
        }
        for (nombre, estado) in [
            (
                "asistente-sin-voz",
                Estado {
                    nivel: 0.0,
                    ..asistente(Fase::Escuchando, "", "")
                },
            ),
            (
                "asistente-escuchando",
                asistente(Fase::Escuchando, "Pon el brillo al cincuenta", ""),
            ),
            (
                "asistente-pensando",
                asistente(
                    Fase::Pensando,
                    "Busca el horario de la biblioteca de Girona",
                    "Buscando en la web…",
                ),
            ),
            (
                "asistente-hecho",
                asistente(Fase::Hecho, "Brillo al 50 %", ""),
            ),
            ("asistente-minimizado", asistente(Fase::Minimizado, "", "")),
            (
                "asistente-respuesta",
                asistente(
                    Fase::Respuesta,
                    "Recordatorio a las 18:00",
                    "Llamar a mamá. Se pierde si cierras la sesión antes.",
                ),
            ),
            (
                "asistente-error",
                asistente(
                    Fase::Error,
                    "El micrófono está silenciado",
                    "Actívalo en Sonido para que pueda oírte.",
                ),
            ),
            (
                "asistente-transcribiendo",
                asistente(Fase::Pensando, "", "Transcribiendo…"),
            ),
            (
                "asistente-titulo-largo",
                asistente(
                    Fase::Escuchando,
                    "WWW: mañana miércoles recuérdame revisar la documentación del proyecto",
                    "Escuchando una petición con un título especialmente largo",
                ),
            ),
            (
                "asistente-respuesta-larga",
                asistente(
                    Fase::Respuesta,
                    "Estas son las tareas pendientes para mañana por la mañana",
                    "Primero, revisa la documentación del proyecto y prepara las notas de la reunión.\nDespués, confirma la hora con el equipo.\n\nTambién puedes reservar un rato para responder los correos pendientes y organizar los archivos de esta semana. Si queda tiempo, revisa el calendario y prepara los materiales de la próxima sesión de trabajo.",
                ),
            ),
            (
                "asistente-respuesta-vacia",
                asistente(Fase::Respuesta, "", ""),
            ),
        ] {
            pinta_varias_a_escala(
                &format!("{nombre}-fraccionario-{etiqueta}"),
                tema,
                vec![estado.clone()],
                &[],
                1.25,
            );
            pinta(&format!("{nombre}-{etiqueta}"), tema, estado, &[]);
        }
        let mut pausado = base(Clase::Player);
        pausado.pausado = true;
        pausado.portada = None;
        pinta(
            &format!("player-sin-portada-{etiqueta}"),
            tema,
            pausado,
            &[(180.0, 55.0)],
        );
    }
}
