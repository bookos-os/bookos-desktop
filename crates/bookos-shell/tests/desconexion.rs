//! «Desconectado» junto al icono de la red y del Bluetooth al perder la
//! conexión. Binario aparte porque cambia el estado del sistema a mitad de la
//! prueba, y en `panel.rs` lo comparten todas.

mod comun;

use bookos_shell::{Config, Shell};

fn sembrar(conectado: bool) {
    let ssid = if conectado { "BookOS prueba" } else { "" };
    bookos_system::seed_test_state(bookos_system::State {
        network: serde_json::json!({"enabled":true,"ssid":ssid,"ethernet":{"connected":false},"networks":[]}),
        bluetooth: serde_json::json!({"present":true,"enabled":true,"devices":[{"connected":conectado}]}),
        ..Default::default()
    });
}

fn ancho(shell: &Shell, nombre: &str) -> f32 {
    shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == nombre)
        .map(|(_, x0, x1)| x1 - x0)
        .expect("el widget está en el panel")
}

/// `BOOKOS_VOLCAR_DESCONEXION=/ruta.png` guarda el panel con el aviso puesto.
#[test]
fn al_desconectar_sale_el_texto_y_luego_se_va() {
    sembrar(true);
    let config = Config {
        derecha: vec!["bluetooth".into(), "red".into()],
        ..Config::default()
    };
    let mut shell = Shell::con_config(comun::ANCHO, 1.0, config);
    shell.refresh();
    let (bt, red) = (ancho(&shell, "bluetooth"), ancho(&shell, "red"));

    sembrar(false);
    assert!(shell.refresh());
    // El texto abre su hueco con un muelle: se mide cuando ha llegado.
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        ancho(&shell, "bluetooth") > bt + 40.0,
        "el Bluetooth enseña el texto"
    );
    assert!(ancho(&shell, "red") > red + 40.0, "la red enseña el texto");
    let alarma = shell.next_tick().expect("pide despertar para quitarlo");
    assert!(alarma <= std::time::Duration::from_secs(3));

    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    comun::volcar("BOOKOS_VOLCAR_DESCONEXION", &buf, w, h);

    std::thread::sleep(alarma);
    assert!(shell.refresh(), "al vencer hay que repintar sin el texto");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(ancho(&shell, "bluetooth"), bt);
    assert_eq!(ancho(&shell, "red"), red);
}
