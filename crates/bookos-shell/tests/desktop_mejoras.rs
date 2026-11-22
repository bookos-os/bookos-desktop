mod comun;
use bookos_shell::{Accion, Emergente, TeclaPulsada, confirmacion::Energia};

#[test]
fn confirmacion_hig_y_cancelacion() {
    let mut shell = comun::shell(1.75);
    shell.abrir(Emergente::confirmar_energia(Energia::Apagar));
    let (w, h) = shell.emergente_buffer_size().unwrap();
    // Canvas alinea el ancho lógico para evitar columnas HiDPI incompletas.
    assert!((w as f32 / 1.75 - 310.0).abs() <= 4.0);
    assert_eq!(h, 364);
    let mut buf = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    comun::volcar("BOOKOS_DUMP_CONFIRMACION", &buf, w, h);
    assert!(matches!(
        shell.emergente_tecla(TeclaPulsada::Intro),
        (true, None)
    ));
    assert!(shell.emergente_nombre().is_none());
    shell.abrir(Emergente::confirmar_energia(Energia::Bloquear));
    shell.emergente_tecla(TeclaPulsada::Tabulador);
    assert!(matches!(
        shell.emergente_tecla(TeclaPulsada::Intro),
        (true, Some(Accion::EnergiaConfirmada(Energia::Bloquear)))
    ));

    // Vuelcos opcionales para inspección visual de todos los estados.
    let volcar_emergente = |var: &str, em: Emergente, teclas: &[TeclaPulsada]| {
        if std::env::var_os(var).is_none() {
            return;
        }
        let mut sh = comun::shell(2.0);
        sh.abrir(em);
        for t in teclas {
            sh.emergente_tecla(t.clone());
        }
        if !teclas.is_empty() {
            std::thread::sleep(bookos_shell::tema::D_HOVER);
        }
        let (bw, bh) = sh.emergente_buffer_size().unwrap();
        let mut b = vec![0; (bw * bh * 4) as usize];
        sh.draw_emergente(&mut b);
        comun::volcar(var, &b, bw, bh);
    };
    volcar_emergente("BOOKOS_DUMP_APAGAR_P1", Emergente::apagar(), &[]);
    volcar_emergente(
        "BOOKOS_DUMP_APAGAR_DEST",
        Emergente::apagar(),
        &[
            TeclaPulsada::Derecha,
            TeclaPulsada::Derecha,
            TeclaPulsada::Derecha,
            TeclaPulsada::Derecha,
        ],
    );
    volcar_emergente(
        "BOOKOS_DUMP_APAGAR_CANCELAR",
        Emergente::apagar(),
        &[TeclaPulsada::Tabulador],
    );
    volcar_emergente(
        "BOOKOS_DUMP_CONF_REINICIAR",
        Emergente::confirmar_energia(Energia::Reiniciar),
        &[],
    );
    volcar_emergente(
        "BOOKOS_DUMP_CONF_SALIR",
        Emergente::confirmar_energia(Energia::CerrarSesion),
        &[],
    );

    if std::env::var_os("BOOKOS_DUMP_CONF_LARGO").is_some() {
        let mut c = bookos_shell::confirmacion::Confirmacion::amplia(
            Energia::Apagar,
            "Hay aplicaciones con cambios sin guardar. Si continúas ahora, se cerrarán de inmediato.",
        );
        c.poner_error("Error: un proceso activo inhibe el apagado.");
        let (cw, ch) = c.size();
        assert!(cw == 420.0 && ch > 208.0);
        let mut sh = comun::shell(2.0);
        sh.abrir(Emergente::confirmar_con(c));
        let (bw, bh) = sh.emergente_buffer_size().unwrap();
        let mut b = vec![0; (bw * bh * 4) as usize];
        sh.draw_emergente(&mut b);
        comun::volcar("BOOKOS_DUMP_CONF_LARGO", &b, bw, bh);
    }
}

#[test]
fn cambiar_dock_redimensiona_y_repinta_a_escala_fraccionaria() {
    let mut shell = comun::shell(1.75);
    for tamano in [32, 80, 50] {
        shell.dock_tamano(tamano);
        assert!(shell.dock_needs_paint());
        let (w, h) = shell.dock_buffer_size();
        let mut buf = vec![0; (w * h * 4) as usize];
        assert!(!shell.draw_dock(&mut buf).is_empty());
        assert!(buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
        assert!(!shell.dock_needs_paint());
        assert_eq!(shell.dock().tamano_actual(), tamano);
    }
}
