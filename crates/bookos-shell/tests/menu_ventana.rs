mod comun;

#[test]
fn menu_ventana_dibuja_y_envia_el_destino_exacto() {
    use bookos_shell::{Accion, Emergente};
    let mut shell = comun::shell(1.0);
    let opciones = vec![
        ("Siempre encima".into(), Accion::VentanaEncima),
        ("Mover a Escritorio 2".into(), Accion::VentanaEscritorio(1)),
        (
            "Mover a HDMI-A-1".into(),
            Accion::VentanaMonitor("HDMI-A-1".into()),
        ),
    ];
    shell.abrir(Emergente::menu_ventana(opciones.clone()));
    let (w, h) = shell.emergente_buffer_size().unwrap();
    let mut pixels = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut pixels);
    comun::volcar("BOOKOS_MENU_VENTANA_PNG", &pixels, w, h);
    assert!(comun::tinta(&pixels, w, h, 0, w) > 0);
    for (i, (_, accion)) in opciones.iter().enumerate() {
        shell.abrir(Emergente::menu_ventana(opciones.clone()));
        assert_eq!(
            shell.emergente_pulsar(100.0, 4.0 + 48.0 * i as f32 + 24.0),
            Some(accion.clone())
        );
    }
}

/// El menú del clic derecho sobre carpetas del escritorio.
///
/// Con `BOOKOS_MENU_ESCRITORIO_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn menu_del_escritorio_se_dibuja() {
    use bookos_shell::{EntradaMenu as E, FilaMenu as F};
    use bookos_shell::{Accion, Emergente};
    let mut shell = comun::shell(2.0);
    let opciones = vec![
        E::Fila(F::new("Abrir", "abrir", Accion::EscritorioEliminarSeleccion)),
        E::Divisor,
        E::Fila(F::new("Mover a la papelera", "papelera", Accion::EscritorioEliminarSeleccion).peligro()),
    ];
    shell.abrir(Emergente::menu_escritorio(opciones, (100.0, 100.0)));
    let (w, h) = shell.emergente_buffer_size().unwrap();
    let mut pixels = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut pixels);
    comun::volcar("BOOKOS_MENU_ESCRITORIO_PNG", &pixels, w, h);
    assert!(comun::tinta(&pixels, w, h, 0, w) > 0);
}

/// «Crear» despliega su lista al pasar el puntero.
///
/// Con `BOOKOS_MENU_CREAR_PNG=/ruta.png` guarda lo que pinta, a 2×.
#[test]
fn menu_del_escritorio_despliega_crear() {
    use bookos_shell::{EntradaMenu as E, FilaMenu as F};
    use bookos_shell::escritorio::NuevoElemento;
    use bookos_shell::{Accion, Emergente};
    let mut shell = comun::shell(2.0);
    let opciones = vec![
        E::Fila(F::submenu(
            "Crear",
            "carpeta-nueva",
            vec![E::Fila(F::new("Carpeta", "carpeta-nueva", Accion::EscritorioNuevo(NuevoElemento::Carpeta)))],
        )),
        E::Divisor,
        E::Fila(F::new("Cambiar fondo…", "fondo", Accion::AbrirCaptura)),
    ];
    shell.abrir(Emergente::menu_escritorio(opciones, (100.0, 100.0)));
    shell.emergente_puntero(Some((20.0, 17.0)));
    let (w, h) = shell.emergente_buffer_size().unwrap();
    let mut pixels = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut pixels);
    comun::volcar("BOOKOS_MENU_CREAR_PNG", &pixels, w, h);
    assert!(comun::tinta(&pixels, w, h, 0, w) > 0);
}

/// El diálogo de color de carpeta se pinta entero, también al crecer con el «+».
#[test]
fn dialogo_de_color_crece_con_el_selector_libre() {
    use bookos_shell::Emergente;
    let mut shell = comun::shell(2.0);
    shell.abrir(Emergente::color_carpeta(&[("Proyectos", None)]));
    let (_, alto) = shell.emergente_buffer_size().unwrap();
    // El «+» es la séptima casilla.
    assert!(
        shell
            .emergente_pulsar(22.0 + 26.0 + 6.0 * 38.0 + 15.0, 22.0 + 168.0 + 15.0)
            .is_none()
    );
    let (w, h) = shell.emergente_buffer_size().unwrap();
    assert!(h > alto, "el lienzo tiene que crecer con el selector");
    let mut pixels = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut pixels);
    assert!(comun::tinta(&pixels, w, h, 0, w) > 0);
}
