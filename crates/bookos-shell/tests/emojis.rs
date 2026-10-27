use bookos_shell::{Accion, Config, Emergente, Shell, TeclaPulsada, tema};

#[test]
fn el_selector_dibuja_emojis_en_ambos_temas_y_entrega_la_secuencia_completa() {
    for modo in [tema::Tema::Claro, tema::Tema::Oscuro] {
        for escala in [1.0, 1.75, 2.0] {
            let mut shell = Shell::con_config(
                1280,
                escala,
                Config {
                    tema: modo,
                    ..Config::default()
                },
            );
            shell.abrir(Emergente::emojis(100.0, 100.0, &[]));
            let (w, h) = shell
                .emergente_buffer_size()
                .expect("el selector está abierto");
            let mut buffer = vec![0; (w * h * 4) as usize];
            shell.draw_emergente(&mut buffer);
            let amarillos = buffer
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[2] > 160 && p[1] > 100 && p[0] < 100 && p[3] > 200)
                .count();
            assert!(
                amarillos > 100,
                "la rejilla debe mostrar las caras en color: {modo:?}, escala {escala}"
            );
            for c in "programador".chars() {
                assert!(shell.emergente_tecla(TeclaPulsada::Caracter(c)).0);
            }
            assert_eq!(
                shell.emergente_tecla(TeclaPulsada::Intro),
                (true, Some(Accion::InsertarEmoji("🧑‍💻".into())))
            );
            assert!(!shell.hay_emergente());
        }
    }
}
