//! Cuánto cuesta un fotograma de la isla.
use bookos_shell::actividad::{Clase, Estado, Fase, ItemCola, Portada};
use bookos_shell::{Config, Shell};
use std::time::Instant;

fn portada() -> Portada {
    let (w, h) = (300u32, 300u32);
    Portada {
        rgba: vec![128; (w * h * 4) as usize],
        width: w,
        height: h,
    }
}

fn estado(clase: Clase, con_portada: bool, cola: usize) -> Estado {
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
        nivel: 0.8,
        aleatorio: true,
        repetir: false,
        portada: con_portada.then(portada),
        metricas: Vec::new(),
        cola: (0..cola)
            .map(|i| ItemCola {
                id: i.to_string(),
                titulo: "t".into(),
                artista: "a".into(),
                duracion_ms: 200_000,
                favorita: i % 2 == 0,
                actual: i == 0,
                portada: None,
            })
            .collect(),
    }
}

fn medir(nombre: &str, e: Estado, clics: &[(f32, f32)]) {
    let mut shell = Shell::con_config(
        1646,
        2.0,
        Config {
            actividades: bookos_shell::ConfigActividades {
                temporizador_siempre: true,
                ..Default::default()
            },
            ..Config::default()
        },
    );
    shell.publicar_actividad(e);
    for (x, y) in clics {
        shell.actividad_pulsar(*x, *y);
    }
    let Some((w, h)) = shell.actividad_buffer_size() else {
        println!("{nombre}: sin isla");
        return;
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_actividad(&mut buf);
    let n = 40;
    let t = Instant::now();
    for _ in 0..n {
        shell.draw_actividad(&mut buf);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!(
        "{nombre:28} {w}x{h}  {ms:6.2} ms/frame  ({:.0} fps)",
        1000.0 / ms
    );
}

fn main() {
    medir("player compacto", estado(Clase::Player, true, 0), &[]);
    medir(
        "player abierto",
        estado(Clase::Player, true, 0),
        &[(180.0, 55.0)],
    );
    medir(
        "player abierto sin portada",
        estado(Clase::Player, false, 0),
        &[(180.0, 55.0)],
    );
    medir(
        "player + cola",
        estado(Clase::Player, true, 4),
        &[(180.0, 55.0), (26.0 + 370.0, 26.0 + 205.0)],
    );
    medir(
        "grabadora abierta",
        estado(Clase::Recorder, false, 0),
        &[(180.0, 55.0)],
    );
    medir(
        "timer abierto",
        estado(Clase::Timer, false, 0),
        &[(180.0, 55.0)],
    );
    // añadido en la 0.2.0 -- las fases con onda de Zenith, que repintan solas.
    for (nombre, fase) in [
        ("zenith escuchando", Fase::Escuchando),
        ("zenith pensando", Fase::Pensando),
        ("zenith minimizado", Fase::Minimizado),
    ] {
        medir(nombre, estado(Clase::Asistente(fase), false, 0), &[]);
    }
}
