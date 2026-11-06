//! El tirador de la lista de Wi-Fi: aparece con más de cuatro redes y sigue a
//! la rueda. Con `BOOKOS_LISTA_PNG=/ruta` guarda `<ruta>-0.png` y `<ruta>-1.png`.
mod comun;

fn redes(n: usize) -> serde_json::Value {
    let lista: Vec<_> = (0..n)
        .map(|i| serde_json::json!({"ssid": format!("Red {i}"), "signal": 80 - i, "active": i == 0, "saved": true, "security": "wpa2", "device": "wlan0"}))
        .collect();
    serde_json::json!({"enabled": true, "networks": lista})
}

fn pinta(shell: &mut bookos_shell::Shell) -> Vec<u8> {
    let (w, h) = shell.emergente_buffer_size().unwrap();
    let mut pixels = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut pixels);
    pixels
}

#[test]
fn la_lista_larga_lleva_tirador_y_la_corta_no() {
    use bookos_shell::Emergente;
    let mut shell = comun::shell(2.0);
    bookos_system::seed_test_state(bookos_system::State {
        network: redes(7),
        ..Default::default()
    });
    shell.abrir(Emergente::red());
    let arriba = pinta(&mut shell);
    let (w, h) = shell.emergente_buffer_size().unwrap();
    comun::volcar("BOOKOS_LISTA_PNG_0", &arriba, w, h);
    shell.emergente_desplazar(0.0, 500.0);
    let abajo = pinta(&mut shell);
    comun::volcar("BOOKOS_LISTA_PNG_1", &abajo, w, h);
    assert_ne!(arriba, abajo);

    bookos_system::seed_test_state(bookos_system::State {
        network: redes(3),
        ..Default::default()
    });
    shell.abrir(Emergente::red());
    let corta = pinta(&mut shell);
    assert!(!corta.is_empty());
}
