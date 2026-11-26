//! Cambio de tema claro/oscuro y refresco de la caché de iconos, en su
//! propio binario de prueba para no competir por los globales de tema e iconos
//! con otras pruebas paralelas.

mod comun;
use comun::shell;

#[test]
fn cambiar_de_tema_limpia_la_cache_y_cambia_iconos_entre_claro_y_oscuro() {
    use bookos_shell::icono;
    use bookos_shell::tema::{self, Tema};

    let antes = (tema::actual(), tema::acento_actual());
    let mut shell = shell(1.0);
    let tiene_tema = |ruta: &std::path::Path, nombres: &[&str]| {
        ruta.components()
            .any(|c| nombres.contains(&c.as_os_str().to_string_lossy().as_ref()))
    };

    shell.aplicar_apariencia(Tema::Claro, antes.1);
    let rev_claro = shell.dock_revision();
    let ruta_claro =
        icono::resolve_icon("network-disconnect").expect("network-disconnect en claro");
    assert!(
        tiene_tema(&ruta_claro, &["breeze", "BookOS-Light"]),
        "en claro debe resolver un icono claro, salió {ruta_claro:?}"
    );
    assert!(!tiene_tema(&ruta_claro, &["breeze-dark", "BookOS-Dark"]));

    shell.aplicar_apariencia(Tema::Oscuro, antes.1);
    let rev_oscuro = shell.dock_revision();
    assert!(rev_oscuro > rev_claro);
    let ruta_oscuro =
        icono::resolve_icon("network-disconnect").expect("network-disconnect en oscuro");
    assert!(
        tiene_tema(&ruta_oscuro, &["breeze-dark", "BookOS-Dark"]),
        "en oscuro debe resolver un icono oscuro, salió {ruta_oscuro:?}"
    );
    assert!(!tiene_tema(&ruta_oscuro, &["breeze", "BookOS-Light"]));

    shell.aplicar_apariencia(Tema::Claro, antes.1);
    assert!(shell.dock_revision() > rev_oscuro);
    let ruta_claro_2 =
        icono::resolve_icon("network-disconnect").expect("network-disconnect de vuelta a claro");
    assert!(
        tiene_tema(&ruta_claro_2, &["breeze", "BookOS-Light"]),
        "al volver a claro no debe quedarse cacheado el oscuro: {ruta_claro_2:?}"
    );

    shell.aplicar_apariencia(antes.0, antes.1);
}
