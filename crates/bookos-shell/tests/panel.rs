//! El panel se pinta de verdad.
//!
//! Es una prueba de humo, no de píxeles: comprobar el color exacto de cada
//! punto ataría el test al tema y saltaría con cada retoque. Lo que se
//! comprueba es lo que de verdad se rompe sin avisar — que `draw_panel` no
//! entra en pánico dentro del proceso del compositor y que deja algo dibujado
//! en la franja donde van los estados.
//!
//! Con `BOOKOS_PANEL_PNG=/ruta/panel.png` además guarda lo pintado, que es la
//! forma más rápida de mirar el panel sin arrancar una sesión.

use bookos_shell::{Config, Shell};

mod comun;
use comun::{ANCHO, distinto_del_fondo, fondo_luma, luma, mezclar, pinta, shell, tinta, volcar};

/// El panel de diagnóstico se pinta y enseña los números que se le pasan.
///
/// Con `BOOKOS_DIAG_PNG=/tmp/d.png` deja el volcado para mirarlo, que es la
/// única forma de comprobar que la letra pequeña se lee sin arrancar la sesión.
#[test]
fn el_panel_de_diagnostico_se_pinta() {
    use bookos_shell::diagnostico::{Datos, Salida};

    let mut shell = shell(1.0);
    assert!(shell.alternar_diagnostico(), "queda puesto al primer toque");
    let cambio = shell.diagnostico_datos(Datos {
        cpu: 12.4,
        gpu: Some(31.0),
        salidas: vec![
            Salida {
                nombre: "eDP-1".into(),
                ancho: 2880,
                alto: 1800,
                escala: 1.75,
                hz: 120.0,
                vrr: true,
                fps: 118.6,
                ms_total: 3.42,
                ms_escena: 2.11,
                ms_render: 1.31,
                ms_peor: 9.87,
                saltados: 42,
                aplazados: 3,
                perdidos: 1,
            },
            Salida {
                nombre: "DP-2".into(),
                ancho: 3840,
                alto: 2160,
                escala: 1.0,
                hz: 60.0,
                fps: 0.0,
                ..Default::default()
            },
        ],
    });
    assert!(cambio, "con números nuevos hay que repintar");

    let (w, h) = shell.diagnostico_buffer_size().expect("está puesto");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let damage = shell.draw_diagnostico(&mut buf);
    assert_eq!(damage.len(), 1, "se repinta entero");
    volcar("BOOKOS_DIAG_PNG", &buf, w, h);

    // Hay tinta en la mitad de abajo: es donde va el bloque de la segunda
    // salida, o sea que la tarjeta creció con ella en vez de recortarla.
    assert!(tinta(&buf, w, h, 0, w) > 20, "el panel salió casi vacío");

    // Los mismos números no repintan: el escritorio quieto no debe despertar
    // para redibujar lo mismo, que es justo lo que este panel sirve para ver.
    let datos = Datos {
        cpu: 12.4,
        gpu: Some(31.0),
        salidas: vec![Salida {
            nombre: "eDP-1".into(),
            ..Default::default()
        }],
    };
    assert!(shell.diagnostico_datos(datos.clone()));
    assert!(
        !shell.diagnostico_datos(datos),
        "sin cambios, no se repinta"
    );

    assert!(!shell.alternar_diagnostico(), "el segundo toque lo quita");
    assert!(shell.diagnostico_buffer_size().is_none());
}

#[test]
fn dibuja_los_estados_en_su_sitio() {
    let (buf, w, h) = pinta(1.0);

    volcar("BOOKOS_PANEL_PNG", &buf, w, h);

    // El nombre va a la izquierda y el reloj al extremo derecho; los estados
    // de en medio dependen de la máquina, así que no se exigen.
    assert!(
        tinta(&buf, w, h, 0, 120) > 0,
        "no se ve el nombre a la izquierda"
    );
    assert!(
        tinta(&buf, w, h, w - 120, w) > 0,
        "no se ve el reloj a la derecha"
    );
}

/// El emergente de Sonido se abre desde el icono del volumen, se dibuja y su
/// deslizador responde al arrastre.
///
/// Con `BOOKOS_SONIDO_PNG=/ruta.png` guarda lo que pinta, para poder mirarlo.
#[test]
fn el_sonido_se_abre_y_su_deslizador_arrastra() {
    let mut shell = shell(1.0);
    // El centro del icono del volumen, sacado de las mismas zonas que usa el
    // compositor para repartir los clics.
    let (_, x0, x1) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "volumen")
        .expect("el volumen está en el panel de serie");
    shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
    assert!(shell.hay_emergente(), "el volumen no abrió su emergente");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_SONIDO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el emergente sale en blanco");

    // Agarrar la píldora por la izquierda y arrastrarla hasta pasarse del
    // borde derecho: el nivel tiene que acabar al máximo, no a medias. La de
    // los altavoces empieza en x = 16 + 4 + 18 + 10 y ocupa de y = 90 a 114.
    shell.emergente_pulsar(60.0, 100.0);
    assert!(shell.emergente_agarrada(), "la píldora no quedó agarrada");
    shell.emergente_puntero(Some((10_000.0, 100.0)));
    assert!(
        shell.soltar().0,
        "soltar no devolvió que había algo agarrado"
    );
    assert!(!shell.emergente_agarrada(), "sigue agarrada tras soltar");
}

/// El emergente del brillo se abre desde su icono y se dibuja.
///
/// Con `BOOKOS_BRILLO_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_brillo_se_abre_desde_su_icono() {
    let mut shell = shell(1.0);
    let Some((_, x0, x1)) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "brillo")
    else {
        // Una máquina sin retroiluminación no dibuja el widget, y entonces no
        // hay nada que abrir: no es un fallo del shell.
        return;
    };
    shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
    assert!(shell.hay_emergente(), "el brillo no abrió su emergente");
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_BRILLO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el emergente sale en blanco");
}

/// Pasar de página en el launchpad cambia la rejilla en un solo repintado y
/// **no** deja al compositor animando.
///
/// La transición deslizada se quitó a propósito (ver `Launchpad::animando`):
/// rasterizar esta rejilla cuesta decenas de milisegundos, así que animarla
/// por CPU bloqueaba el hilo del compositor durante toda la transición y se
/// notaba hasta en el cursor. Lo que hay que garantizar es lo contrario de lo
/// que se comprobaba antes: que se pinta **una** vez y se acaba.
///
/// Con `BOOKOS_PAGINA_PNG=/ruta.png` guarda la página nueva ya pintada.
#[test]
fn el_launchpad_cambia_de_pagina_en_un_solo_repintado() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::launchpad((ANCHO as f32, 800.0)));
    assert!(shell.hay_emergente(), "no abrió el launchpad");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut primera = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut primera);

    // Un gesto de sobra para pasar el umbral de 20 px.
    if !shell.emergente_desplazar(60.0, 0.0) {
        // Con menos de treinta aplicaciones instaladas no hay segunda página.
        return;
    }
    assert!(
        !shell.emergente_animando(),
        "el cambio de página no debe animar: rasterizar la rejilla por fotograma \
         bloquea el hilo del compositor"
    );
    assert!(
        shell.emergente_needs_paint(),
        "la página nueva tiene que repintar el buffer una vez"
    );

    let mut segunda = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut segunda);
    volcar("BOOKOS_PAGINA_PNG", &segunda, w, h);
    assert_ne!(primera, segunda, "la rejilla no cambió de página");

    // Y una vez pintada, el compositor puede dormirse: no queda nada en marcha.
    assert!(
        !shell.emergente_needs_paint(),
        "tras pintar la página nueva no queda nada que repintar"
    );
}

/// El buscador declara el alto que de verdad dibuja, y sus filas se pulsan
/// donde se ven.
///
/// El espacio entre el campo y la lista cuenta tanto en el alto como en el
/// hit-test, para que el puntero señale la fila que se ve.
#[test]
fn el_buscador_pulsa_donde_dibuja() {
    use bookos_shell::TeclaPulsada;
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::buscador());
    for c in "sh".chars() {
        shell.emergente_tecla(TeclaPulsada::Caracter(c));
    }
    let ((_, alto), _, _) = shell.emergente_geometria().unwrap();

    // Cuántas filas hay: lo que sobra del campo y el divisor, en filas de 54.
    let filas = (alto - 68 - 1 - 8 - 10) / 54;
    assert!(
        filas >= 1,
        "«sh» tiene que encontrar algo: el alto es {alto}"
    );
    assert_eq!(
        alto,
        68 + 1 + 8 + filas * 54 + 10,
        "el alto declarado no cuadra con lo que apila la vista"
    );

    // Y el borde de arriba de la primera fila es suyo, no del campo de texto.
    // Se comprueba por el hover, que usa el mismo `fila_en` que el clic:
    // `emergente_puntero` devuelve `true` solo cuando la fila señalada
    // **cambia**, así que partiendo de ninguna, el orden de estas dos dice
    // exactamente dónde empieza la lista.
    assert!(
        !shell.emergente_puntero(Some((100.0, 76.0))),
        "el campo de texto no es ninguna fila: no puede cambiar el realce"
    );
    assert!(
        shell.emergente_puntero(Some((100.0, 77.0))),
        "el primer píxel tras la separación señala la fila 0"
    );
}

/// Ninguna superficie del shell puede pedirle al compositor un tamaño lógico
/// que no dé píxeles enteros a la escala de la pantalla.
///
/// Si no cuadra, Smithay reescala la textura y el filtrado bilineal emborrona
/// todo el contenido: es lo que hacía que las tarjetas se vieran desenfocadas a
/// escala 1,75 mientras el panel salía nítido.
#[test]
fn las_superficies_caen_en_pixel_entero() {
    for escala in [1.0, 1.25, 1.5, 1.75, 2.0] {
        let mut shell = shell(escala);
        for (nombre, emergente) in [
            ("red", bookos_shell::Emergente::red()),
            ("bluetooth", bookos_shell::Emergente::bluetooth()),
            ("notificaciones", bookos_shell::Emergente::notificaciones()),
            ("centro", bookos_shell::Emergente::centro()),
            ("energia", bookos_shell::Emergente::energia()),
            ("calendario", bookos_shell::Emergente::calendario()),
            ("menu", bookos_shell::Emergente::menu()),
            ("acerca", bookos_shell::Emergente::acerca()),
        ] {
            shell.abrir(emergente);
            let ((lw, lh), _, _) = shell.emergente_geometria().expect("hay emergente");
            let (bw, bh) = shell.emergente_buffer_size().expect("hay buffer");
            assert_eq!(
                bw,
                (lw as f32 * escala).round() as u32,
                "{nombre} a {escala}: el ancho lógico {lw} no cuadra con el buffer {bw}"
            );
            assert_eq!(
                bh,
                (lh as f32 * escala).round() as u32,
                "{nombre} a {escala}: el alto lógico {lh} no cuadra con el buffer {bh}"
            );
            // Y el físico que pide el compositor tiene que ser entero.
            let pedido = lw as f32 * escala;
            assert!(
                (pedido - pedido.round()).abs() < 0.001,
                "{nombre} a {escala}: pide {pedido} px, que no es entero"
            );
        }
    }
}

/// El panel a la escala de un portátil HiDPI, que es donde se vería un
/// rasterizado a 1× ampliado después: el buffer tiene que salir con los píxeles
/// de verdad, no con los lógicos estirados.
///
/// Con `BOOKOS_PANEL_175_PNG=/ruta.png` lo guarda.
#[test]
fn el_panel_a_escala_fraccionaria_se_rasteriza_a_pixeles_reales() {
    let mut shell = shell(1.75);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    volcar("BOOKOS_PANEL_175_PNG", &buf, w, h);
    // El buffer es físico: 1,75 veces el ancho lógico.
    assert_eq!(w, (ANCHO as f32 * 1.75).round() as u32);
    assert!(
        h >= 56,
        "el panel de 32 lógicos son 56 físicos, y salen {h}"
    );
}

/// La otra batería, la de trazo monocromo, en el panel.
///
/// Con `BOOKOS_PANEL_SIMPLE_PNG=/ruta.png` guarda el panel con ella puesta,
/// para poder compararla al lado de la que dibuja el pictograma relleno.
#[test]
fn la_bateria_simple_se_pinta() {
    let mut config = Config::default();
    config.derecha[0] = "bateria-simple".into();
    let mut shell = Shell::con_config(ANCHO, 1.0, config);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    volcar("BOOKOS_PANEL_SIMPLE_PNG", &buf, w, h);
    assert!(
        tinta(&buf, w, h, w / 2, w) > 0,
        "la mitad derecha sale vacía"
    );
}

/// Cada widget del panel abre su tarjeta al pulsarlo.
///
/// Es lo que se ve desde fuera: que exista el módulo no significa que el clic
/// llegue. Aquí se pulsa el centro de cada zona y se mira qué se abrió.
#[test]
fn todos_los_widgets_abren_su_tarjeta() {
    let esperado = [
        ("bateria", "energia"),
        ("bluetooth", "bluetooth"),
        ("red", "red"),
        ("volumen", "sonido"),
        ("brillo", "brillo"),
        ("notificaciones", "notificaciones"),
        ("control", "centro"),
        ("reloj", "calendario"),
    ];
    let mut shell = shell(1.0);
    let zonas = shell.zonas_panel();
    for (widget, tarjeta) in esperado {
        let Some((_, x0, x1)) = zonas.iter().find(|(n, _, _)| *n == widget).copied() else {
            // Un widget que no se dibuja en esta máquina no tiene zona.
            continue;
        };
        shell.cerrar_emergente();
        shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
        assert_eq!(
            shell.emergente_nombre(),
            Some(tarjeta),
            "pulsar «{widget}» no abrió «{tarjeta}»"
        );
    }
}

/// Las tarjetas de conectividad, con lo que haya en esta máquina.
///
/// Con `BOOKOS_RED_PNG` y `BOOKOS_BT_PNG` las guarda.
#[test]
fn las_tarjetas_de_conectividad_se_pintan() {
    for (emergente, var) in [
        (bookos_shell::Emergente::red(), "BOOKOS_RED_PNG"),
        (bookos_shell::Emergente::bluetooth(), "BOOKOS_BT_PNG"),
        (
            bookos_shell::Emergente::notificaciones(),
            "BOOKOS_NOTIF_PNG",
        ),
        (bookos_shell::Emergente::centro(), "BOOKOS_CENTRO_PNG"),
        (bookos_shell::Emergente::energia(), "BOOKOS_ENERGIA_PNG"),
    ] {
        let mut shell = shell(1.0);
        shell.abrir(emergente);
        let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_emergente(&mut buf);
        volcar(var, &buf, w, h);
        assert!(tinta(&buf, w, h, 0, w) > 0, "{var} sale en blanco");
    }
}

/// Pulsar lo gris de la píldora del Wi-Fi abre su lista **dentro** de la
/// Estación de Control, como la bandeja de Plasma, y Esc vuelve.
///
/// Con `BOOKOS_CENTRO_WIFI_PNG` y `BOOKOS_CENTRO_BT_PNG` las guarda.
#[test]
fn la_estacion_abre_las_listas_en_el_sitio() {
    // Margen, cabecera y hueco hasta las píldoras, y su mitad.
    let y = 16.0 + 52.0 + 12.0 + 26.0;
    for (x, var) in [
        (110.0, "BOOKOS_CENTRO_WIFI_PNG"),
        (290.0, "BOOKOS_CENTRO_BT_PNG"),
    ] {
        let mut shell = shell(1.0);
        shell.abrir(bookos_shell::Emergente::centro());
        let antes = shell.emergente_buffer_size().expect("tiene superficie");
        assert_eq!(
            shell.emergente_pulsar(x, y),
            None,
            "{var}: no abre otra tarjeta"
        );
        assert!(shell.hay_emergente(), "{var}: la estación sigue abierta");
        let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
        // Sin servicio de red o Bluetooth no hay lista que abrir: la píldora no
        // hace nada y la estación se queda igual. No es un fallo del shell.
        if (w, h) == antes {
            continue;
        }
        assert_eq!(w, antes.0, "{var}: el ancho no cambia al entrar");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_emergente(&mut buf);
        volcar(var, &buf, w, h);
        let (consumida, _) = shell.emergente_tecla(bookos_shell::TeclaPulsada::Escape);
        assert!(
            consumida && shell.hay_emergente(),
            "{var}: Esc vuelve, no cierra"
        );
        assert_eq!(
            shell.emergente_buffer_size(),
            Some(antes),
            "{var}: y vuelve entera"
        );
        // La flecha de la cabecera hace lo mismo que Esc.
        shell.emergente_pulsar(x, y);
        assert_eq!(shell.emergente_pulsar(32.0, 16.0 + 16.0), None);
        assert_eq!(
            shell.emergente_buffer_size(),
            Some(antes),
            "{var}: la flecha vuelve"
        );
    }
}

/// El centro de notificaciones con avisos dentro: repetidos, críticos, con
/// progreso y con acciones.
///
/// Con `BOOKOS_NOTIF_LLENO_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_centro_de_notificaciones_con_avisos_se_pinta() {
    use bookos_shell::notificaciones::{Accion, Notificacion};

    let mut shell = shell(1.0);
    let acc = |c: &str, e: &str| Accion {
        clave: c.into(),
        etiqueta: e.into(),
    };
    for (id, app, resumen, cuerpo, critica, acciones, progreso) in [
        (
            1,
            "KDE Connect",
            "Instagram",
            "pero el problema no es ese es que no se ve bien la notificacion",
            false,
            vec![acc("r", "Responder"), acc("l", "Leído")],
            None,
        ),
        (
            2,
            "KDE Connect",
            "WhatsApp",
            "¿Quedamos mañana?",
            false,
            vec![],
            None,
        ),
        (
            3,
            "Firefox",
            "Descarga terminada",
            "bookos-desktop.tar.gz",
            false,
            vec![],
            Some(60),
        ),
        (
            4,
            "Sistema",
            "Batería crítica",
            "Conecta el cargador",
            true,
            vec![],
            None,
        ),
    ] {
        shell.notificar(
            Notificacion::nueva_con_datos(
                id,
                app.into(),
                resumen.into(),
                cuerpo.into(),
                "",
                critica,
                acciones,
                progreso,
                false,
            ),
            -1,
        );
    }
    shell.abrir(bookos_shell::Emergente::notificaciones());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_NOTIF_LLENO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");

    // Pulsar la fila de Instagram (la última: la lista va del más nuevo al más
    // viejo) la despliega y la tarjeta crece.
    assert!(shell.emergente_pulsar(100.0, 250.0).is_none());
    let (w2, h2) = shell.emergente_buffer_size().expect("tiene superficie");
    assert!(h2 > h, "desplegar no hizo crecer la tarjeta");
    let mut buf = vec![0u8; (w2 * h2 * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_NOTIF_DESPLEGADO_PNG", &buf, w2, h2);
}

/// El aviso de volumen, con la cápsula del diseño.
///
/// Con `BOOKOS_OSD_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_osd_se_pinta() {
    let mut shell = shell(1.0);
    shell.mostrar_osd("volumen-medio", Some(60), None);
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el aviso sale en blanco");

    // El de la cámara, con su icono propio.
    for (icono, texto, var) in [
        ("camara", "Cámara activada", "BOOKOS_OSD_CAMARA_PNG"),
        (
            "camara-desactivado",
            "Cámara desactivada",
            "BOOKOS_OSD_CAMARA_OFF_PNG",
        ),
    ] {
        shell.mostrar_osd(icono, None, Some(texto.into()));
        let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_osd(&mut buf);
        volcar(var, &buf, w, h);
    }

    shell.mostrar_osd(
        "touchpad-desactivado",
        None,
        Some("Touchpad desactivado".into()),
    );
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_TOUCHPAD_OFF_PNG", &buf, w, h);

    // Y el de estado, que lleva texto en vez de barra.
    shell.mostrar_osd("touchpad", None, Some("Touchpad desactivado".into()));
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_TEXTO_PNG", &buf, w, h);

    // El del cambio de escritorio es de esa misma clase, y es lo único que se
    // ve al cambiar a uno vacío: sin ventanas no hay nada que deslizar.
    shell.mostrar_osd("escritorio-2", None, Some("Escritorio 2".into()));
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_ESCRITORIO_PNG", &buf, w, h);
    assert!(
        tinta(&buf, w, h, 0, w) > 0,
        "el aviso del escritorio sale en blanco"
    );
}

/// «Acerca de este PC» se dibuja con los datos de esta máquina.
///
/// Con `BOOKOS_ACERCA_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_acerca_se_pinta() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::acerca());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_ACERCA_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");
}

/// El menú del clic derecho sobre un icono del dock.
///
/// Con `BOOKOS_MENU_DOCK_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_clic_derecho_del_dock_abre_su_menu() {
    let mut shell = shell(1.0);
    // Sobre una aplicación abierta, para que salga también "Cerrar".
    shell.dock_ventanas(&["org.kde.dolphin".to_string()]);
    let centro_y = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
    // El primero es el launchpad, que no tiene menú; el segundo sí.
    let x_launchpad = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
    assert!(
        !shell.dock_menu(x_launchpad, centro_y),
        "el launchpad no debe tener menú contextual"
    );
    let x = x_launchpad + 2.0 * (bookos_shell::DOCK_ICON + 14.0);
    assert!(shell.dock_menu(x, centro_y), "no abrió el menú del dock");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_MENU_DOCK_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el menú sale en blanco");
}

/// Una aplicación abierta sin lanzador entra en el dock y se ve, que es lo que
/// hace falta para poder fijarla con el clic derecho.
///
/// Con `BOOKOS_DOCK_ABIERTA_PNG=/ruta.png` guarda el dock con ella dentro.
#[test]
fn una_ventana_sin_lanzador_sale_en_el_dock() {
    let mut shell = shell(1.0);
    let (w0, _) = shell.dock_buffer_size();
    assert!(
        shell.dock_ventanas(&["org.gnome.Calculator".to_string()]),
        "abrir una ventana desconocida tiene que cambiar el dock"
    );
    let (w, h) = shell.dock_buffer_size();
    assert!(w > w0, "el dock no creció: {w} no es mayor que {w0}");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut buf);
    volcar("BOOKOS_DOCK_ABIERTA_PNG", &buf, w, h);

    // Y al cerrarla, el dock vuelve a su ancho.
    assert!(shell.dock_ventanas(&[]));
    assert_eq!(shell.dock_buffer_size().0, w0);
}

/// Lanzar un icono del dock sin ventana lo hace rebotar, sin cambiar el
/// tamaño del dock.
///
/// Con `BOOKOS_DOCK_REBOTE_PNG=/ruta.png` guarda el dock en lo alto del salto.
#[test]
fn lanzar_desde_el_dock_hace_rebotar_el_icono() {
    let mut shell = shell(1.0);
    let Some(exec) = shell
        .dock()
        .items()
        .iter()
        .find(|i| !i.abierta() && !i.exec().is_empty())
        .map(|i| i.exec().to_string())
    else {
        return;
    };
    let tam = shell.dock_buffer_size();
    let (w, h) = tam;
    let mut quieto = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut quieto);

    assert!(shell.dock_lanzando(&exec));
    assert!(shell.dock_animando(), "rebotando hay que seguir pintando");
    // Medio periodo: lo alto del salto.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        shell.dock_buffer_size(),
        tam,
        "el rebote no cambia el tamaño"
    );
    let mut arriba = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut arriba);
    volcar("BOOKOS_DOCK_REBOTE_PNG", &arriba, w, h);
    assert_ne!(quieto, arriba, "el icono no se ha movido");
}

/// El emergente de la energía se abre desde la batería y trae sus perfiles.
///
/// Con `BOOKOS_ENERGIA_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn la_energia_se_abre_desde_la_bateria() {
    let mut shell = shell(1.0);
    let Some((_, x0, x1)) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "bateria")
    else {
        // Un sobremesa no dibuja el widget de la batería.
        return;
    };
    shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
    assert!(shell.hay_emergente(), "la batería no abrió su emergente");
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_ENERGIA_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el emergente sale en blanco");
}

/// El centro de control, que es la tarjeta con más colores propios del shell:
/// círculos de acento, conmutadores apagados y las tarjetas interiores.
#[test]
fn el_centro_de_control_se_pinta() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::centro());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_CENTRO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el centro sale en blanco");
}

/// La separación entre widgets del panel, que es la tolerancia con la que se
/// reparten los clics.
fn view_hueco() -> f32 {
    20.0
}

/// Las zonas que el shell declara para el hit-test tienen que caer donde de
/// verdad se ha pintado cada widget.
///
/// Es la comprobación que hace fiable a `Widget::ancho`, que es una estimación
/// y no una medida: si un widget declara de menos, pierde clics en su borde; si
/// declara de más, se los roba al vecino. Aquí se pinta el panel y se mira si
/// hay tinta dentro de cada zona declarada.
#[test]
fn cada_widget_recibe_los_clics_donde_se_dibuja() {
    let escala = 1.0;
    let mut shell = shell(escala);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);

    let zonas = shell.zonas_panel();
    assert!(!zonas.is_empty(), "el panel no declara ninguna zona");

    // La zona se comprueba con la tolerancia del hueco que separa los widgets.
    // `Widget::ancho` es una estimación —el texto lo mide iced al pintar, y
    // aquí hace falta antes— así que el error se acumula de derecha a
    // izquierda: con el reloj y la batería cambiando de anchura según la hora y
    // el porcentaje, la zona del cuarto widget puede irse unos píxeles. Lo que
    // tiene que cumplirse es que el clic caiga en **su** widget, y para eso
    // basta con que el dibujo esté dentro de la zona más el margen que da el
    // hueco. Exigirlo al píxel hacía fallar el test una vez de cada tres.
    let margen = view_hueco() / 2.0;
    for (nombre, x0, x1) in zonas {
        assert!(x1 > x0, "la zona de {nombre} está vacía: {x0}..{x1}");
        let a = (x0 - margen).max(0.0) as u32;
        let b = ((x1 + margen).max(0.0) as u32).min(w);
        let dentro = tinta(&buf, w, h, a, b);
        assert!(
            dentro > 0,
            "la zona de {nombre} ({a}..{b}) está en blanco: se declara donde no se pinta"
        );
    }
}

/// El panel se pinta a la escala del monitor, así que el buffer crece con
/// ella. Que el dibujo siga cayendo dentro es justo lo que se rompía cuando
/// se confundían píxeles lógicos y físicos.
#[test]
fn aguanta_una_escala_fraccionaria() {
    let (buf, w, h) = pinta(1.75);
    assert_eq!((w, h), ((ANCHO as f32 * 1.75) as u32, 56));
    assert!(tinta(&buf, w, h, 0, 200) > 0, "no se ve nada a esa escala");
}

/// El menú se abre desde el logo, se recorre y se cierra.
///
/// Con `BOOKOS_MENU_PNG=/ruta/menu.png` guarda el menú con una entrada
/// señalada.
#[test]
fn el_menu_se_abre_se_recorre_y_se_cierra() {
    use bookos_shell::TeclaPulsada;

    let mut shell = shell(1.0);
    assert!(!shell.hay_emergente());

    // Pulsar el nombre de la izquierda lo abre.
    shell.panel_pulsado(20.0, 16.0);
    assert!(
        shell.hay_emergente(),
        "pulsar el logo tiene que abrir el menú"
    );

    let ((w, h), _ancla, _) = shell.emergente_geometria().unwrap();
    // Más ancho que los 210 del plasmoide desde que las filas llevan icono.
    assert_eq!(w, 236, "el ancho del menú");
    assert!(h > 0);

    // Bajar señala la primera entrada, y eso obliga a repintar.
    assert!(shell.emergente_tecla(TeclaPulsada::Abajo).0);
    // El realce tarda lo que dice el token de hover; sin esperarlo, el volcado
    // enseña la fila todavía sin encender.
    std::thread::sleep(bookos_shell::tema::D_HOVER);
    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_MENU_PNG", &buf, bw, bh);
    // El acento del hover es sólido, así que tiene que haber píxeles azules.
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el menú sale vacío");

    // Y Esc lo cierra.
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(!shell.hay_emergente(), "Esc tiene que cerrar el menú");
}

/// Pulsar una entrada del menú devuelve su acción y lo cierra.
#[test]
fn elegir_del_menu_lo_cierra() {
    let mut shell = shell(1.0);
    shell.panel_pulsado(20.0, 16.0);

    // La primera entrada, "Acerca de este PC": empieza tras el margen de 2 px
    // y mide 26 de alto.
    let accion = shell.emergente_pulsar(100.0, 2.0 + 13.0);
    assert!(accion.is_some(), "el centro de la primera fila no responde");
    assert!(
        !shell.hay_emergente(),
        "elegir algo tiene que cerrar el menú"
    );

    // Y en un separador no hay nada que elegir, así que el menú sigue abierto.
    shell.panel_pulsado(20.0, 16.0);
    assert_eq!(shell.emergente_pulsar(100.0, 2.0 + 26.0 + 3.0), None);
    assert!(
        shell.hay_emergente(),
        "fallar la puntería no cierra el menú"
    );
}

/// El calendario se abre desde el reloj del centro.
///
/// Con `BOOKOS_CALENDARIO_PNG=/ruta/cal.png` lo guarda.
#[test]
fn el_calendario_se_abre_desde_el_reloj() {
    let mut shell = shell(1.0);
    // El extremo derecho, que es donde va el reloj.
    shell.panel_pulsado(ANCHO as f32 - 30.0, 16.0);
    assert!(shell.hay_emergente(), "pulsar el reloj abre el calendario");

    let ((w, _h), _, _) = shell.emergente_geometria().unwrap();
    assert_eq!(w, 336, "el ancho es el de las demás tarjetas del panel");

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_CALENDARIO_PNG", &buf, bw, bh);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el calendario sale vacío");

    // Pulsar el logo con el calendario abierto cambia al menú, no lo cierra.
    shell.panel_pulsado(20.0, 16.0);
    assert_eq!(shell.emergente_geometria().unwrap().0.0, 236);
}

/// El compositor compara el nombre de la zona pulsada con el nombre interno
/// de la tarjeta abierta. No siempre son iguales: si se compara `bateria` con
/// `energia`, el segundo clic cree que son widgets distintos y vuelve a abrir
/// la misma tarjeta en vez de cerrarla.
#[test]
fn cada_widget_declara_el_nombre_real_de_su_emergente() {
    let shell = shell(1.0);
    let zonas = shell.zonas_panel();

    let centro = |nombre: &str| {
        let (_, x0, x1) = zonas
            .iter()
            .find(|(n, _, _)| *n == nombre)
            .expect("el widget está en el panel de prueba");
        (x0 + x1) / 2.0
    };

    assert_eq!(
        shell.objetivo_emergente_panel(centro("bateria")),
        Some(("bateria", "energia"))
    );
    assert_eq!(
        shell.objetivo_emergente_panel(centro("reloj")),
        Some(("reloj", "calendario"))
    );
}

/// El launchpad se abre y se pinta con las aplicaciones de esta máquina.
///
/// Es una prueba de humo a propósito: cuántas aplicaciones hay depende del
/// sistema donde corra, así que lo que se comprueba es que se abre, tiene el
/// tamaño que dice y deja píxeles. La lógica de búsqueda y de páginas se prueba
/// aparte, con una lista fija.
///
/// Con `BOOKOS_LAUNCHPAD_PNG=/ruta/lp.png` lo guarda.
#[test]
fn el_launchpad_se_abre_y_se_pinta() {
    use bookos_shell::{Emergente, TeclaPulsada};

    let mut shell = shell(1.0);
    shell.abrir(Emergente::launchpad((ANCHO as f32, 800.0)));
    assert!(shell.hay_emergente());

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el launchpad sale vacío");

    // El volcado compone lo que verá el usuario, no solo este buffer: el velo
    // y el realce los pinta el compositor, así que un PNG del buffer a secas
    // enseñaría una rejilla flotando en la nada y no diría nada del aspecto.
    let velo = shell.emergente_velo().expect("el launchpad pide velo");
    shell.emergente_puntero(Some((100.0, 200.0)));
    let realce = shell.emergente_realce();
    let mut compuesto = vec![0u8; (bw * bh * 4) as usize];
    for p in compuesto.as_chunks_mut::<4>().0 {
        // El velo va sobre negro, que es el escritorio vacío.
        p[0] = (velo[2] * velo[3] * 255.0) as u8;
        p[1] = (velo[1] * velo[3] * 255.0) as u8;
        p[2] = (velo[0] * velo[3] * 255.0) as u8;
        p[3] = 255;
    }
    if let Some((rx, ry, rw, rh, marco)) = realce {
        let (mut rbuf, rw_px, rh_px) = {
            let (w, h) = (rw as u32, rh as u32);
            (vec![0u8; (w * h * 4) as usize], w, h)
        };
        shell.draw_realce(&mut rbuf, rw, rh, marco);
        mezclar(
            &mut compuesto,
            bw,
            &rbuf,
            rw_px,
            rh_px,
            rx as u32,
            ry as u32,
        );
    }
    mezclar(&mut compuesto, bw, &buf, bw, bh, 0, 0);
    volcar("BOOKOS_LAUNCHPAD_PNG", &compuesto, bw, bh);

    // Escribir no lo cierra; Esc con búsqueda solo la limpia, y el segundo sí.
    assert!(shell.emergente_tecla(TeclaPulsada::Caracter('k')).0);
    assert!(shell.hay_emergente());
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(
        shell.hay_emergente(),
        "el primer Esc solo limpia la búsqueda"
    );
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(!shell.hay_emergente(), "el segundo Esc sí cierra");
}

/// Señalar un icono tiene que **verse**. El hit-test ya se prueba aparte; lo
/// que aquí se comprueba es que el resaltado llega hasta los píxeles, que es
/// donde se queda un `hover` que se guarda pero no se dibuja.
///
/// Con `BOOKOS_DOCK_PNG=/ruta/dock.png` guarda el dock con el primer icono
/// señalado.
#[test]
fn el_icono_señalado_se_ve_distinto() {
    let mut shell = shell(1.0);
    // Con una ventana abierta, para ver también el indicador en el volcado. Va
    // **antes** de medir: abrir una aplicación que no está fijada ensancha el
    // dock, y un búfer del tamaño viejo se rechaza sin pintar nada.
    shell.dock_ventanas(&["org.kde.dolphin".to_string()]);
    let (w, h) = shell.dock_buffer_size();

    let mut apagado = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut apagado);

    let centro = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
    assert!(
        shell.dock_hover(Some((centro, centro))),
        "señalar un icono nuevo tiene que pedir repintado"
    );
    // La placa entra en 120 ms: sin esperarlos, lo que se pinta es el primer
    // fotograma de la animación, casi idéntico al icono apagado.
    std::thread::sleep(bookos_shell::tema::D_HOVER);
    let mut señalado = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut señalado);
    volcar("BOOKOS_DOCK_PNG", &señalado, w, h);
    assert!(
        señalado.as_chunks::<4>().0.iter().any(|p| p[3] != 0),
        "el dock señalado sale transparente"
    );

    assert_ne!(
        apagado, señalado,
        "el icono señalado sale idéntico al normal"
    );
    assert!(
        !shell.dock_hover(Some((centro, centro))),
        "señalar lo mismo dos veces no puede costar un repintado"
    );
    assert!(shell.dock_hover(None), "salir del dock tiene que apagarlo");
}

/// La foto de perfil del bloqueo sale de `panel.conf` y se recorta en círculo.
///
/// Con `BOOKOS_BLOQUEO_FOTO_PNG=/ruta.png` guarda lo que sale. La foto se
/// genera aquí —un cuadrado de color— en vez de traerse de la máquina: el test
/// tiene que valer en una donde nadie ha puesto ninguna.
#[test]
fn el_bloqueo_enseña_la_foto_de_la_configuracion() {
    let ruta = std::env::temp_dir().join("bookos-test-avatar.png");
    let foto = image::RgbaImage::from_fn(240, 160, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 200, 255])
    });
    foto.save(&ruta).expect("se puede escribir en el temporal");

    // Sin la animación de entrada: el test pinta un único fotograma en el
    // instante del bloqueo, y ahí el avatar todavía está a alfa cero. Es una
    // configuración de verdad —la que deja puesta «reducir movimiento»—, no un
    // apaño para el test.
    let config = Config {
        avatar: Some(ruta.to_string_lossy().into_owned()),
        bloqueo: bookos_shell::ConfigBloqueo {
            animaciones: false,
            ..Default::default()
        },
        ..Config::default()
    };
    let mut shell = Shell::con_config(ANCHO, 1.0, config);
    let pantalla = (1645.0, 1029.0);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    let (w, h) = shell
        .bloqueo_buffer_size()
        .expect("hay superficie de bloqueo");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf);
    volcar("BOOKOS_BLOQUEO_FOTO_PNG", &buf, w, h);

    // El avatar es la mancha opaca de la columna central en la mitad de abajo:
    // se busca en vez de calcular su `y`, que depende de dónde acabe el texto.
    let alfa = |x: u32, y: u32| buf[((y * w + x) * 4 + 3) as usize];
    let centro_x = w / 2;
    let filas: Vec<u32> = ((h / 2)..(h * 4 / 5))
        .filter(|y| alfa(centro_x, *y) > 200)
        .collect();
    let (arriba, abajo) = (
        *filas.first().expect("no hay avatar en la columna central"),
        *filas.last().expect("no hay avatar en la columna central"),
    );
    let diametro = abajo - arriba;
    assert!(
        diametro > 60,
        "el avatar mide {diametro} px de alto: la foto no se pintó"
    );
    // Y es un círculo: la esquina de su cuadrado tiene que estar vacía. Sin el
    // recorte, la foto salía rectangular y llenaba también las esquinas.
    let radio = diametro / 2;
    let esquina = (
        centro_x + (radio as f32 * 0.78) as u32,
        arriba + (radio as f32 * 0.22) as u32,
    );
    assert!(
        alfa(esquina.0, esquina.1) < 40,
        "la esquina del avatar está pintada: la foto no se recortó en círculo"
    );
    let _ = std::fs::remove_file(&ruta);
}

/// La pantalla de bloqueo, pintada sobre el fondo del escritorio.
///
/// Con `BOOKOS_BLOQUEO_PNG=/ruta.png` guarda lo que sale, para compararlo con
/// el diseño. Va al tamaño lógico de la pantalla real —1645×1029, que es
/// 2880×1800 a escala 1,75— porque las medidas del diseño son absolutas: a la
/// mitad, el reloj y el avatar ocupan el doble de lo que les toca.
#[test]
fn el_bloqueo_se_pinta_con_reloj_avatar_y_campo() {
    use bookos_shell::bloqueo::{Estado, Medio};

    let pantalla = (1645.0, 1029.0);
    // Igual que el de la foto: un solo fotograma, y con la entrada animada ese
    // fotograma es el primero, o sea la pantalla entera a alfa cero.
    let mut config = Config::default();
    config.bloqueo.animaciones = false;
    let mut shell = Shell::con_config(ANCHO, 1.0, config);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    assert!(shell.esta_bloqueado());
    if let Some(b) = shell.bloqueo_mut() {
        b.escritos = 6;
    }

    let (w, h) = shell
        .bloqueo_buffer_size()
        .expect("hay superficie de bloqueo");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf);
    volcar("BOOKOS_BLOQUEO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "no se ha pintado nada");
    // El reloj vive en la franja de arriba: si no hay tinta ahí, se ha vuelto a
    // quedar sin dibujar, que es justo lo que pasó la primera vez.
    let alto_reloj = h / 4;
    let franja: u32 = (0..alto_reloj)
        .map(|y| {
            (0..w)
                .filter(|x| {
                    let i = ((y * w + x) * 4) as usize;
                    buf[i].max(buf[i + 1]).max(buf[i + 2]) > 60
                })
                .count() as u32
        })
        .sum();
    assert!(franja > 0, "el reloj no se dibujó");

    // Y con algo sonando, la tarjeta se añade sin cambiar el tamaño.
    if let Some(b) = shell.bloqueo_mut() {
        b.estado = Estado::Fallo;
        b.medios.push(Medio {
            fuente: None,
            caratula: None,
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Recording".into(),
            detalle: "0:37".into(),
            color: iced_core::Color::from_rgb(0.9, 0.2, 0.2),
            progreso: Some(0.2),
            posicion: Some(37),
            duracion: Some(185),
            reproduciendo: true,
        });
    }
    let mut buf2 = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf2);
    volcar("BOOKOS_BLOQUEO_MEDIOS_PNG", &buf2, w, h);
    assert_eq!(
        shell.bloqueo_buffer_size(),
        Some((w, h)),
        "la tarjeta cambió el tamaño de la superficie"
    );

    // Y con el menú de apagado desplegado, que es la otra variante del diseño.
    if let Some(b) = shell.bloqueo_mut() {
        b.menu = true;
    }
    let mut buf3 = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf3);
    volcar("BOOKOS_BLOQUEO_MENU_PNG", &buf3, w, h);

    shell.desbloquear();
    assert!(!shell.esta_bloqueado());
}

/// El silencio puesto: la tarjeta crece y enseña las cuatro duraciones.
///
/// Con `BOOKOS_NOTIF_DND_PNG=/ruta.png` la guarda. Es el estado que no se ve al
/// abrir la tarjeta y donde estaba el error de altura: el hueco reservado tiene
/// que cuadrar con lo que se pinta.
#[test]
fn el_no_molestar_enseña_sus_duraciones() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::notificaciones());
    let (_, alto_antes) = shell.emergente_buffer_size().expect("tiene superficie");
    // El interruptor vive en la fila de «No molestar», a la derecha: tarjeta,
    // margen, relleno del grupo y aire hasta la mitad del interruptor; y en
    // alto, cabecera, grupo, la fila vacía de la lista, el divisor y media
    // fila del bloque.
    shell.emergente_pulsar(
        336.0 - 16.0 - 6.0 - 8.0 - 23.0,
        16.0 + 32.0 + 12.0 + 6.0 + 52.0 + 16.5 + 26.0,
    );
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    assert!(h > alto_antes, "con el silencio puesto la tarjeta crece");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_NOTIF_DND_PNG", &buf, w, h);
    // La franja de abajo es la de los chips: si el hueco reservado sobrara,
    // saldría en negro.
    assert!(tinta(&buf, w, h, h - 30, w) > 0, "los chips no se pintan");
}

/// Las zonas de clic del panel caen donde está el dibujo.
///
/// Es el test que faltaba cuando las zonas se estimaban a 7,3 px por carácter:
/// el reloj declaraba 95 px y ocupaba 73, y ese error se acumulaba hacia la
/// izquierda hasta dejar a la batería 10 px fuera de su zona. Se comprueba
/// contra el reloj porque es el único widget que se dibuja en cualquier
/// máquina.
#[test]
fn las_zonas_del_panel_cuadran_con_lo_pintado() {
    let mut shell = shell(1.0);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);

    let fondo = fondo_luma(&buf, w, h);
    let hay_tinta = |x: u32| (0..h).any(|y| distinto_del_fondo(luma(&buf, w, x, y), fondo));
    let (_, x0, x1) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "reloj")
        .expect("el reloj sale siempre");
    // Toda la tinta de la derecha del panel tiene que estar dentro de la zona
    // del reloj: si sobresale, su ancho declarado se queda corto.
    let ultima = (0..w).rev().find(|x| hay_tinta(*x)).expect("hay reloj");
    let primera_del_reloj = (x0 as u32..w).find(|x| hay_tinta(*x)).expect("hay reloj");
    assert!(
        (ultima as f32) <= x1,
        "el reloj se pinta hasta {ultima} y su zona acaba en {x1}"
    );
    assert!(
        primera_del_reloj as f32 >= x0,
        "el reloj empieza en {primera_del_reloj}, antes de su zona ({x0})"
    );

    // Y las zonas se tocan entre sí: un hueco entre dos es un sitio del panel
    // donde pulsar no hace nada.
    let mut zonas = shell.zonas_panel();
    zonas.sort_by(|a, b| a.1.total_cmp(&b.1));
    for par in zonas.windows(2) {
        let (izq, der) = (&par[0], &par[1]);
        assert!(
            (der.1 - izq.2).abs() < 0.01,
            "hueco muerto entre «{}» y «{}»: {} .. {}",
            izq.0,
            der.0,
            izq.2,
            der.1
        );
    }
}

/// El indicador de escritorios, con el dato que solo el compositor conoce.
///
/// Con `BOOKOS_ESCRITORIOS_PNG=/ruta.png` guarda el panel con él puesto.
///
/// Antes de que llegue el dato del compositor el widget ya enseña los dos
/// escritorios de la configuración de BookOS: así el panel no cambia de ancho
/// al entrar en la sesión.
#[test]
fn el_indicador_de_escritorios_se_pinta_y_responde() {
    let mut shell = shell(1.0);
    assert!(
        shell
            .zonas_panel()
            .iter()
            .any(|(n, x0, x1)| *n == "escritorios" && x1 - x0 > 0.01),
        "al arrancar ya tiene que enseñar los dos de la configuración"
    );

    assert!(shell.escritorios(2, 4), "el dato nuevo pide repintar");
    assert!(!shell.escritorios(2, 4), "el mismo dato, no");

    let (_, x0, x1) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "escritorios")
        .expect("ya tiene zona");
    assert!(
        x1 - x0 > 20.0,
        "cuatro puntos ocupan más de 20 px: {}",
        x1 - x0
    );

    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    volcar("BOOKOS_ESCRITORIOS_PNG", &buf, w, h);
    assert!(
        tinta(&buf, w, h, x0 as u32, x1 as u32) > 0,
        "el indicador no se ve en su zona"
    );

    // Pulsar el primer punto lleva al primer escritorio, no abre ninguna
    // tarjeta: es el único widget del panel que responde por su cuenta.
    let accion = shell.panel_pulsado(x0 + 2.0, 8.0);
    assert_eq!(accion, Some(bookos_shell::Accion::Escritorio(0)));
    assert!(!shell.hay_emergente(), "no debe abrir ninguna tarjeta");

    let paso = (x1 - x0) / 4.0;
    let accion = shell.panel_pulsado(x0 + paso * 2.0 + 1.0, 8.0);
    assert_eq!(accion, Some(bookos_shell::Accion::VistaEscritorios));
}

#[test]
fn la_vista_de_escritorios_se_pinta_y_conserva_las_acciones_de_gestion() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::escritorios(
        (ANCHO as f32, 720.0),
        0,
        vec!["Trabajo".into(), "Personal".into()],
    ));
    assert_eq!(shell.emergente_nombre(), Some("escritorios"));
    assert_eq!(shell.escritorios_miniaturas().len(), 2);

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_VISTA_ESCRITORIOS_PNG", &buf, bw, bh);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "la vista sale vacía");

    // El + vive suelto en el extremo derecho de la franja, a 24 px del borde:
    // con 1280 px de ancho cae sobre x=1230, centrado con las miniaturas.
    assert_eq!(
        shell.emergente_pulsar(1230.0, 105.0),
        Some(bookos_shell::Accion::CrearEscritorio)
    );
    assert!(shell.hay_emergente(), "crear no debe cerrar la vista");
    assert!(shell.actualizar_vista_escritorios(
        0,
        vec!["Trabajo".into(), "Personal".into(), "Escritorio 3".into()]
    ));
    assert_eq!(shell.escritorios_miniaturas().len(), 3);
}

/// El conmutador de Alt+Tab: la fila de celdas con la elegida realzada.
///
/// Con `BOOKOS_CONMUTADOR_PNG=/ruta.png` guarda lo que pinta. Los iconos se
/// resuelven del tema del sistema, así que en una máquina pelada saldrán huecos
/// —lo que se comprueba aquí es la tarjeta, la selección y el rótulo.
#[test]
fn el_conmutador_se_pinta_y_recorre() {
    let mut shell = shell(1.0);
    let apps: Vec<_> = ["firefox", "konsole", "dolphin"]
        .into_iter()
        .map(|id| bookos_shell::entrada_de_ventana(id, ""))
        .collect();
    assert!(
        shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Aplicaciones,
            apps,
            (1280.0, 720.0),
        ),
        "con tres celdas se abre"
    );

    let (w, h) = shell.conmutador_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_conmutador(&mut buf);
    volcar("BOOKOS_CONMUTADOR_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el conmutador sale en blanco");

    // El primer paso del compositor lleva de la actual a la anterior.
    shell.conmutador_mover(1);
    assert_eq!(shell.cerrar_conmutador(), Some(1));
    assert!(!shell.hay_conmutador(), "cerrarlo lo suelta");

    // Con una sola celda se abre igual —se ve qué hay abierto— y se queda en ella.
    let una = vec![bookos_shell::entrada_de_ventana("firefox", "")];
    assert!(
        shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Aplicaciones,
            una,
            (1280.0, 720.0),
        ),
        "con una sola también se abre"
    );
    shell.conmutador_mover(1);
    assert_eq!(shell.cerrar_conmutador(), Some(0));
}

/// Dos ventanas de la **misma** aplicación: el caso que dejó Alt+Tab muerto.
///
/// Repiten icono y lo único que las distingue es el título, así que se comprueba
/// que la tarjeta se abre igual y que el índice elegido es el de la celda, no
/// algo derivado de la aplicación —que sería el mismo para las dos.
#[test]
fn dos_ventanas_de_la_misma_app_se_conmutan() {
    let mut shell = shell(1.0);
    let ventanas = vec![
        bookos_shell::entrada_de_ventana("org.kde.konsole", "~/proyecto"),
        bookos_shell::entrada_de_ventana("org.kde.konsole", "ssh servidor"),
    ];
    assert!(
        shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Ventanas,
            ventanas,
            (1280.0, 720.0),
        ),
        "dos ventanas de la misma aplicación tienen que abrir el conmutador"
    );

    let (w, h) = shell.conmutador_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_conmutador(&mut buf);
    volcar("BOOKOS_CONMUTADOR_VENTANAS_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "sale en blanco");

    // El compositor aplica el primer Tab después de abrir.
    shell.conmutador_mover(1);
    assert_eq!(shell.cerrar_conmutador(), Some(1));
}

/// Un título vacío no puede dejar la celda sin rótulo: se cae al nombre de la
/// aplicación. Hay clientes que tardan en poner el título.
#[test]
fn sin_titulo_se_usa_el_nombre_de_la_app() {
    let con = bookos_shell::entrada_de_ventana("konsole", "  ");
    assert!(!con.nombre.is_empty());
}

/// El menú de apagado del bloqueo **se despliega**: no aparece de golpe.
///
/// Se mide sobre el buffer recién dibujado y no sobre un volcado, y se compara
/// el primer fotograma con el de después de la animación: los tiempos exactos
/// dependen de lo que tarde el rasterizado: el primer frame puede haber
/// avanzado ya unos píxeles, especialmente al ejecutar otros tests en paralelo.
/// Lo importante es que el menú crezca hasta su altura completa.
#[test]
fn el_menu_del_bloqueo_se_despliega() {
    let pantalla = (1645.0, 1029.0);
    let mut shell = shell(1.0);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    let (w, h) = shell
        .bloqueo_buffer_size()
        .expect("hay superficie de bloqueo");

    // El centro del botón de los tres puntos, en la esquina de abajo.
    shell.bloqueo_pulsado(50.0, pantalla.1 - 50.0);

    let alto_menu = |shell: &mut Shell| -> u32 {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_bloqueo(&mut buf);
        let alfa = |y: u32| buf[((y * w + 60) * 4 + 3) as usize];
        // Solo por encima del botón: es donde se despliega el menú.
        let filas: Vec<u32> = (h / 2..h - 90).filter(|y| alfa(*y) > 30).collect();
        match (filas.first(), filas.last()) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    };

    let inicial = alto_menu(&mut shell);
    std::thread::sleep(std::time::Duration::from_millis(250));
    let final_ = alto_menu(&mut shell);
    assert!(
        inicial < final_,
        "el menú no se desplegó: {inicial} → {final_}"
    );
    assert!(
        final_ > 100,
        "el menú no acabó de desplegarse: {final_} px de alto"
    );
}

/// El diálogo del botón de encendido se pinta y sus cinco opciones responden.
///
/// Con `BOOKOS_APAGAR_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_dialogo_de_energia_se_pinta_y_responde() {
    use bookos_shell::{Accion, TeclaPulsada};

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::apagar());
    // Es un diálogo: lleva velo, o lo de debajo no se apaga.
    assert!(shell.emergente_velo().is_some(), "el diálogo sale sin velo");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_APAGAR_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el diálogo sale en blanco");

    // Cuatro a la derecha llegan a «Apagar», y ahí Intro apaga.
    for _ in 0..4 {
        assert!(shell.emergente_tecla(TeclaPulsada::Derecha).0);
    }
    let (_, accion) = shell.emergente_tecla(TeclaPulsada::Intro);
    assert!(
        matches!(accion, Some(Accion::Lanzar(ref cmd)) if cmd.contains("poweroff")),
        "Intro sobre «Apagar» tiene que apagar: {accion:?}"
    );
    // Y ejecutar cierra el diálogo.
    assert!(!shell.hay_emergente());

    // Esc lo cierra sin hacer nada.
    shell.abrir(bookos_shell::Emergente::apagar());
    let (repintar, accion) = shell.emergente_tecla(TeclaPulsada::Escape);
    assert!(repintar && accion.is_none());
    assert!(!shell.hay_emergente());
}

/// La tarjeta de notificaciones enseña la cola, y pulsar una la cierra.
///
/// Con `BOOKOS_NOTIF_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn las_notificaciones_se_listan_y_se_cierran() {
    use bookos_shell::Accion;
    use bookos_shell::notificaciones::Notificacion;

    let mut shell = shell(1.0);
    for (i, (app, resumen, cuerpo, critica)) in [
        (
            "Firefox",
            "Descarga terminada",
            "bookos-desktop.tar.gz",
            false,
        ),
        ("Batería", "Batería baja", "Quedan 7 minutos", true),
        (
            "Konsole",
            "Compilación lista",
            "cargo build: 0 errores",
            false,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        shell.notificar(
            Notificacion::nueva(
                i as u32 + 1,
                app.into(),
                resumen.into(),
                cuerpo.into(),
                "",
                critica,
            ),
            -1,
        );
    }
    assert_eq!(shell.notificaciones().len(), 3);

    shell.abrir_de_widget("notificaciones");
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_NOTIF_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");

    // Pulsar la primera fila pide cerrar **esa**, y la tarjeta se queda abierta.
    // La lista empieza bajo la cabecera: margen (21) + cabecera (40).
    let accion = shell.emergente_pulsar(60.0, 75.0);
    let Some(Accion::CerrarNotificacion(id)) = accion else {
        panic!("pulsar una notificación tiene que cerrarla: {accion:?}");
    };
    assert_eq!(id, 3, "la más nueva va arriba");
    assert!(shell.hay_emergente(), "cerrar una no cierra la tarjeta");

    // El shell la retira y el contador baja.
    assert!(shell.cerrar_notificacion(id));
    assert_eq!(shell.notificaciones().len(), 2);

    // Y «Borrar todo» las quita todas, devolviendo a quién hay que avisar.
    let ids = shell.borrar_notificaciones();
    assert_eq!(ids.len(), 2);
    assert!(shell.notificaciones().is_empty());
}

/// El aviso con cuerpo y botones —el de KDE Connect— cabe entero en su tarjeta.
///
/// Con `BOOKOS_TOAST_ACCIONES_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_aviso_con_botones_se_pinta() {
    use bookos_shell::notificaciones::{Accion, Notificacion};

    let accion = |clave: &str, etiqueta: &str| Accion {
        clave: clave.into(),
        etiqueta: etiqueta.into(),
    };
    let mut shell = shell(1.0);
    shell.notificar(
        Notificacion::nueva_con_datos(
            1,
            "KDE Connect".into(),
            "Instagram".into(),
            "pero el problema no es ese es que no se ve bien la notificacion".into(),
            "",
            false,
            vec![
                accion("responder", "Responder"),
                accion("leido", "Marcar como leído"),
            ],
            None,
            false,
        ),
        -1,
    );
    let (w, h) = shell.toast_buffer_size(1).expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_toast(1, &mut buf);
    volcar("BOOKOS_TOAST_ACCIONES_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el aviso sale en blanco");
}

/// El aviso de una notificación se pinta arriba a la derecha y se va solo.
///
/// Con `BOOKOS_TOAST_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_aviso_de_notificacion_se_pinta_y_se_descarta() {
    use bookos_shell::notificaciones::Notificacion;

    let mut shell = shell(1.0);
    shell.notificar(
        Notificacion::nueva(
            1,
            "Firefox".into(),
            "Descarga terminada".into(),
            "bookos-desktop.tar.gz".into(),
            "",
            false,
        ),
        -1,
    );
    let (w, h) = shell.toast_buffer_size(1).expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_toast(1, &mut buf);
    volcar("BOOKOS_TOAST_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el aviso sale en blanco");
    assert!(shell.toasts_vivos());

    // Pulsarlo lo manda a irse, pero la notificación se queda en la lista:
    // despedir el aviso no es haberla atendido.
    assert!(shell.descartar_toast(1));
    assert!(
        shell
            .toasts_quedan()
            .first()
            .is_some_and(|q| *q <= bookos_shell::toast::SALIDA),
        "descartar tiene que dejarle solo la salida"
    );
    assert_eq!(shell.notificaciones().len(), 1);
}

/// Los avisos se apilan, el más nuevo arriba; si llega uno de más, el más
/// antiguo se va, y uno que se actualiza se queda en su sitio.
#[test]
fn los_avisos_se_apilan_y_el_que_sobra_se_va() {
    use bookos_shell::notificaciones::Notificacion;
    use bookos_shell::toast::MAXIMO_A_LA_VISTA;

    let mut shell = shell(1.0);
    let n = |id: u32| {
        Notificacion::nueva(
            id,
            "App".into(),
            format!("Aviso {id}"),
            String::new(),
            "",
            false,
        )
    };
    for id in 1..=MAXIMO_A_LA_VISTA as u32 {
        shell.notificar(n(id), -1);
    }
    assert_eq!(shell.toasts_ids(), vec![3, 2, 1], "el más nuevo va arriba");

    shell.notificar(n(4), -1);
    assert_eq!(shell.toasts_ids(), vec![4, 3, 2, 1]);
    assert!(
        shell
            .toasts_quedan()
            .last()
            .is_some_and(|q| *q <= bookos_shell::toast::SALIDA),
        "el que sobra tiene que estar saliendo, no quitarse de golpe"
    );

    // `replaces_id`: no vuelve arriba.
    shell.notificar(n(2), -1);
    assert_eq!(shell.toasts_ids(), vec![4, 3, 2, 1]);
}

/// Abrir una tarjeta despide los avisos, y con ella abierta no salen nuevos:
/// caían encima de la de notificaciones y tapaban «No molestar».
#[test]
fn una_tarjeta_abierta_aparta_los_avisos() {
    use bookos_shell::notificaciones::Notificacion;

    let mut shell = shell(1.0);
    let n = |id: u32, critica: bool| {
        Notificacion::nueva(id, "App".into(), "Aviso".into(), String::new(), "", critica)
    };
    shell.notificar(n(1, false), -1);
    shell.abrir_de_widget("notificaciones");
    assert!(
        shell
            .toasts_quedan()
            .iter()
            .all(|q| *q <= bookos_shell::toast::SALIDA),
        "abrir la tarjeta tiene que mandar a irse los avisos"
    );

    shell.notificar(n(2, false), -1);
    assert!(
        !shell.toasts_ids().contains(&2),
        "no debe salir encima de la tarjeta"
    );
    assert_eq!(shell.notificaciones().len(), 2, "pero sí se guarda");
    shell.notificar(n(3, true), -1);
    assert!(shell.toasts_ids().contains(&3), "una crítica sale igual");
}

/// «No molestar» sobrevive a cerrar la tarjeta, y mientras está puesto una
/// notificación normal no saca aviso pero una crítica sí.
#[test]
fn el_silencio_sobrevive_y_deja_pasar_lo_critico() {
    use bookos_shell::notificaciones::Notificacion;

    let mut shell = shell(1.0);
    shell.abrir_de_widget("notificaciones");
    // El interruptor de «No molestar», en la fila de su bloque.
    let (w, _) = shell.emergente_geometria().unwrap().0;
    shell.emergente_pulsar(w as f32 - 40.0, 160.0);
    assert!(
        shell.notificaciones_silenciadas(),
        "pulsar el interruptor tiene que silenciar"
    );

    // Cerrar la tarjeta no lo olvida: es lo que pasaba cuando el silencio vivía
    // dentro de la emergente, que se destruye al cerrarla.
    shell.cerrar_emergente();
    assert!(
        shell.notificaciones_silenciadas(),
        "el silencio se ha perdido"
    );

    // Silenciado, una normal se guarda sin avisar…
    shell.notificar(
        Notificacion::nueva(
            1,
            "Firefox".into(),
            "Descarga".into(),
            String::new(),
            "",
            false,
        ),
        -1,
    );
    assert!(!shell.toasts_vivos(), "con silencio no debe salir el aviso");
    assert_eq!(shell.notificaciones().len(), 1, "pero sí se guarda");

    // …y una crítica sale igual: para eso es crítica.
    shell.notificar(
        Notificacion::nueva(
            2,
            "Batería".into(),
            "Batería baja".into(),
            String::new(),
            "",
            true,
        ),
        -1,
    );
    assert!(
        shell.toasts_vivos(),
        "una crítica tiene que salir aunque haya silencio"
    );
}

/// El launchpad con carpetas: la tapa con miniaturas, y entrar y salir.
///
/// Con `BOOKOS_CARPETAS_PNG=/ruta.png` guarda la rejilla con la carpeta, y con
/// `BOOKOS_DENTRO_PNG=/ruta.png` lo que se ve dentro.
#[test]
fn el_launchpad_dibuja_carpetas() {
    // La configuración va a un directorio de usar y tirar: abrir el launchpad
    // lee `launchpad.conf`, y el test no puede depender de lo que tenga quien
    // compile ni escribirle nada.
    let dir = std::env::temp_dir().join("bookos-test-carpetas");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bookos")).unwrap();
    std::fs::write(
        dir.join("bookos/launchpad.conf"),
        "Utilidades = konsole, kate, dolphin\n",
    )
    .unwrap();
    // SAFETY: este test corre en su propio binario y nadie más toca la
    // variable; se restaura al terminar.
    let antes = std::env::var_os("XDG_CONFIG_HOME");
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::launchpad((1280.0, 800.0)));
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_CARPETAS_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el launchpad sale en blanco");

    // La carpeta va la primera de la rejilla: pulsarla entra dentro y no lanza
    // nada. Se pulsa el centro de su celda y no un punto fijo: el (40, 120)
    // que había caía en el hueco bajo el buscador desde que pasó a 4 unidades.
    let (cx, cy) = shell.launchpad_celda(0).expect("la carpeta tiene celda");
    let accion = shell.emergente_pulsar(cx, cy);
    assert!(accion.is_none(), "pulsar agarra, no lanza: {accion:?}");
    let (_, accion) = shell.soltar();
    assert!(accion.is_none(), "entrar en una carpeta no es una acción");
    assert!(shell.hay_emergente(), "y no cierra el launchpad");

    let mut dentro = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut dentro);
    volcar("BOOKOS_DENTRO_PNG", &dentro, w, h);
    assert_ne!(buf, dentro, "dentro de la carpeta se ve lo mismo que fuera");

    // Elegir un color tiñe la carpeta. El selector nace cerrado, así que hacen
    // falta dos clics: el chip lo despliega y la tira da el color. Se comprueba
    // **fuera**, que es donde se ve la tapa: se elige, se sale y se repinta.
    let (chip, tono) = shell
        .launchpad_selector(200.0)
        .expect("dentro de una carpeta hay selector");
    shell.emergente_pulsar(chip.0, chip.1);
    let (_, accion) = shell.soltar();
    assert!(accion.is_none(), "desplegar el selector no es una acción");
    shell.emergente_pulsar(tono.0, tono.1);
    let (_, accion) = shell.soltar();
    assert!(accion.is_none(), "elegir un color tampoco");
    shell.emergente_tecla(bookos_shell::TeclaPulsada::Escape);
    let mut teñida = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut teñida);
    volcar("BOOKOS_COLOR_PNG", &teñida, w, h);
    assert_ne!(
        buf, teñida,
        "la carpeta se pinta igual con color que sin él"
    );

    // El Esc de arriba ya sacó de la carpeta —y no cerró el launchpad, que es
    // lo que se comprueba aquí—; el siguiente sí lo cierra.
    assert!(
        shell.hay_emergente(),
        "salir de la carpeta no puede cerrarlo"
    );
    shell.emergente_tecla(bookos_shell::TeclaPulsada::Escape);
    assert!(!shell.hay_emergente());

    match antes {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
}

/// La barra de título se pinta y sus tres botones caen donde dice el hit-test.
///
/// Con `BOOKOS_BARRA_PNG=/ruta.png` guarda lo que sale, que es la forma de
/// mirar los glifos sin arrancar una sesión.
#[test]
fn la_barra_de_titulo_pinta_sus_botones() {
    use bookos_shell::decoracion::{Boton, Estado, boton_en};

    const ANCHO_BARRA: f32 = 640.0;
    let mut shell = shell(1.0);
    // Con el cursor sobre el cierre: es el estado que más cosas dibuja —el
    // círculo rojo y la tinta que se lee encima— y el que se rompería sin que
    // ningún otro test se enterase.
    let estado = Estado {
        pestanas: Vec::new(),
        pestana_activa: 0,
        titulo: "Konsole — bash".into(),
        activa: true,
        maximizada: false,
        señalado: Some(Boton::Cerrar),
        pulsado: None,
    };
    assert!(
        shell.barra_preparar(1, ANCHO_BARRA, estado),
        "hay que pintarla"
    );
    let (w, h) = shell.barra_buffer_size(1).expect("la barra recién creada");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let damage = shell.draw_barra(1, &mut buf);
    assert_eq!(damage.len(), 1, "la barra se repinta entera");
    volcar("BOOKOS_BARRA_PNG", &buf, w, h);
    assert_eq!(
        buf[3], 0,
        "restaurada, la esquina superior izquierda debe ser transparente"
    );

    // Los tres botones dejan tinta en su zona.
    for (boton, nombre) in [
        (Boton::Minimizar, "minimizar"),
        (Boton::Maximizar, "maximizar"),
        (Boton::Cerrar, "cerrar"),
    ] {
        // El centro de la zona que el hit-test le asigna, buscado desde el
        // borde derecho: así el test comprueba a la vez el dibujo y la
        // geometría que usa el compositor para repartir los clics.
        let x = (0..ANCHO_BARRA as u32)
            .rev()
            .find(|x| boton_en(ANCHO_BARRA, *x as f32, h as f32 / 2.0) == Some(boton))
            .unwrap_or_else(|| panic!("el hit-test no encuentra {nombre}"));
        assert!(
            tinta(&buf, w, h, x.saturating_sub(10), x) > 0,
            "no se ve el botón de {nombre}"
        );
    }

    // Y el título, en el centro.
    assert!(
        tinta(&buf, w, h, w / 2 - 60, w / 2 + 60) > 0,
        "no se ve el título"
    );

    // Repetir con el mismo estado no obliga a repintar: es lo que evita
    // rasterizar una barra por ventana en cada frame.
    let igual = Estado {
        pestanas: Vec::new(),
        pestana_activa: 0,
        titulo: "Konsole — bash".into(),
        activa: true,
        maximizada: false,
        señalado: Some(Boton::Cerrar),
        pulsado: None,
    };
    assert!(
        !shell.barra_preparar(1, ANCHO_BARRA, igual),
        "repinta de más"
    );

    // Maximizada vuelve a ser rectangular para no enseñar el fondo por las
    // esquinas de la pantalla.
    let maximizada = Estado {
        pestanas: Vec::new(),
        pestana_activa: 0,
        titulo: "Konsole — bash".into(),
        activa: true,
        maximizada: true,
        señalado: None,
        pulsado: None,
    };
    assert!(shell.barra_preparar(1, ANCHO_BARRA, maximizada));
    let mut buf_max = vec![0u8; (w * h * 4) as usize];
    shell.draw_barra(1, &mut buf_max);
    assert_ne!(buf_max[3], 0, "maximizada no debe conservar el recorte");
}

/// El buscador se abre, escribe, se recorre con las flechas y lanza con Intro.
///
/// Con `BOOKOS_BUSCADOR_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_buscador_busca_y_lanza() {
    use bookos_shell::{Accion, TeclaPulsada};

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::buscador());
    assert!(shell.hay_emergente());
    assert_eq!(shell.emergente_nombre(), Some("buscador"));
    assert!(
        !shell.emergente_usa_cristal(),
        "una tarjeta opaca no debe dejar un halo del fondo en las esquinas"
    );

    // Vacío enseña como mucho las sugerencias de lo más usado, que dependen
    // de `usos.conf` de quien pruebe: aquí solo se fija el ancho.
    let ((w, _), _, _) = shell.emergente_geometria().unwrap();
    assert_eq!(w, 720);

    // «sh» encuentra al menos el ejecutable del PATH, que existe en cualquier
    // Linux: la tarjeta pasa de ser solo el campo (68) a tener lista.
    for c in "sh".chars() {
        assert!(shell.emergente_tecla(TeclaPulsada::Caracter(c)).0);
    }
    let ((_, alto), _, _) = shell.emergente_geometria().unwrap();
    assert!(alto > 68, "la lista no apareció al escribir");

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_BUSCADOR_PNG", &buf, bw, bh);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el buscador sale vacío");

    // El alto con el que el compositor lo **coloca** no cambia con lo escrito:
    // es el del campo, que queda centrado mientras los resultados crecen hacia
    // abajo. Ver `Emergente::alto_estable`.
    let estable = shell.emergente_geometria().unwrap().2;
    assert_eq!(
        estable,
        Some(68),
        "Spotlight tiene que colocarse por el campo centrado, no por la lista"
    );

    // Intro lanza lo elegido y cierra.
    let (repintar, accion) = shell.emergente_tecla(TeclaPulsada::Intro);
    assert!(repintar);
    assert!(
        matches!(accion, Some(Accion::Lanzar(_))),
        "Intro tiene que lanzar lo elegido, y devolvió {accion:?}"
    );
    assert!(!shell.hay_emergente(), "lanzar cierra el buscador");
}

/// Esc borra lo escrito antes de cerrar: lo primero que se quiere deshacer es
/// la búsqueda, no la ventana.
#[test]
fn el_escape_del_buscador_borra_antes_de_cerrar() {
    use bookos_shell::TeclaPulsada;

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::buscador());
    shell.emergente_tecla(TeclaPulsada::Caracter('s'));
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(shell.hay_emergente(), "el primer Esc solo borra lo escrito");
    shell.emergente_tecla(TeclaPulsada::Escape);
    assert!(!shell.hay_emergente(), "el segundo Esc cierra");
}

/// La capa de captura se pinta: la barra de modos y la pastilla con las
/// medidas, cada una en su superficie. El velo con su agujero no sale aquí: lo
/// compone el compositor, y se comprueba allí.
///
/// Con `BOOKOS_CAPTURA_PNG=/ruta.png` guarda la barra, y con
/// `BOOKOS_CAPTURA_PASTILLA_PNG` la pastilla, para mirarlas contra el resto del
/// escritorio.
#[test]
fn la_capa_de_captura_se_pinta() {
    use bookos_shell::{PiezaCaptura, TeclaPulsada};

    let pantalla = (1280.0, 800.0);
    let mut shell = shell(1.0);
    shell.abrir_captura(pantalla);
    assert!(shell.hay_captura());
    assert_eq!(
        shell.captura_origen(PiezaCaptura::Pastilla),
        None,
        "sin recuadro la pastilla no va en ningún sitio"
    );

    // Un recuadro marcado a mano, que es lo que enseña de verdad la capa.
    shell.captura_pulsar(240.0, 180.0);
    shell.captura_puntero(880.0, 560.0);

    for (pieza, variable) in [
        (PiezaCaptura::Barra, "BOOKOS_CAPTURA_PNG"),
        (PiezaCaptura::Pastilla, "BOOKOS_CAPTURA_PASTILLA_PNG"),
    ] {
        assert!(shell.captura_needs_paint(pieza), "{pieza:?} sin pintar");
        let (w, h) = shell.captura_buffer_size(pieza).expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_captura(pieza, &mut buf);
        volcar(variable, &buf, w, h);
        assert!(tinta(&buf, w, h, 0, w) > 0, "{pieza:?} sale en blanco");
        assert!(!shell.captura_needs_paint(pieza));
    }

    let r = shell.captura_marcado().expect("hay recuadro");
    assert_eq!((r.x, r.y, r.w, r.h), (240.0, 180.0, 640.0, 380.0));
    // La pastilla va centrada bajo el recuadro.
    let (px, py) = shell
        .captura_origen(PiezaCaptura::Pastilla)
        .expect("con recuadro la pastilla se coloca");
    assert!(py > r.y + r.h, "la pastilla tiene que ir debajo");
    assert_eq!(
        px + bookos_shell::captura::PASTILLA_ANCHO / 2.0,
        r.x + r.w / 2.0
    );

    // Mover sin cambiar lo que se ve no repinta nada: es lo que hace que
    // arrastrar no rasterice en cada evento del ratón.
    shell.captura_puntero(880.2, 560.3);
    assert!(!shell.captura_needs_paint(PiezaCaptura::Barra));
    assert!(!shell.captura_needs_paint(PiezaCaptura::Pastilla));
    shell.captura_puntero(900.0, 560.3);
    assert!(!shell.captura_needs_paint(PiezaCaptura::Barra));
    assert!(shell.captura_needs_paint(PiezaCaptura::Pastilla));

    // Soltar no captura: lo marcado se queda esperando, e Intro lo copia.
    shell.captura_soltar();
    assert!(shell.captura_marcado().is_some(), "soltar deja el recuadro");
    assert!(matches!(
        shell.captura_tecla(TeclaPulsada::Intro),
        bookos_shell::Tecla::Hacer(bookos_shell::Accion::Capturar {
            x: 240,
            y: 180,
            ancho: 660,
            alto: 380,
            guardar: false
        })
    ));
}

/// La entrada del bloqueo se **mueve**: no basta con que exista el código de la
/// animación, tiene que cambiar lo que se pinta entre un instante y otro.
///
/// Se comprueba midiendo tinta en tres momentos. El primer fotograma va a alfa
/// cero a propósito —por eso el test de la foto de arriba desactiva las
/// animaciones—, así que arrancar con la pantalla en blanco no es el fallo: el
/// fallo sería quedarse ahí.
#[test]
fn la_entrada_del_bloqueo_se_anima() {
    use std::time::Duration;

    let pantalla = (1645.0, 1029.0);
    let mut shell = Shell::new(ANCHO, 1.0);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    assert!(
        shell.bloqueo_animando(),
        "recién echado tiene que haber animación en marcha"
    );

    let (w, h) = shell
        .bloqueo_buffer_size()
        .expect("hay superficie de bloqueo");
    let mut medir = || {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_bloqueo(&mut buf);
        comun::tinta(&buf, w, h, 0, w)
    };

    let principio = medir();
    std::thread::sleep(Duration::from_millis(400));
    let medio = medir();
    std::thread::sleep(Duration::from_millis(500));
    let final_ = medir();

    assert_ne!(
        principio, medio,
        "el bloqueo no cambió en los primeros 400 ms: la entrada está congelada"
    );
    assert!(
        final_ > 0,
        "al terminar la entrada el bloqueo tiene que estar pintado"
    );
    assert!(
        !shell.bloqueo_animando(),
        "pasados los 760 ms de ENTRADA_TOTAL la animación tiene que haber acabado"
    );
}

/// Comprobación visual del solape del bloqueo con la tarjeta de medios, a la
/// resolución lógica real del portátil: 1280×800 (2240×1400 físicos a escala
/// 1,75). Con `BOOKOS_BLOQUEO_1280_PNG=/ruta.png` se puede mirar el PNG.
#[test]
fn el_bloqueo_no_se_solapa_a_1280x800() {
    use bookos_shell::bloqueo::{Estado, Medio};

    let pantalla = (1280.0, 800.0);
    let mut config = Config::default();
    config.bloqueo.animaciones = false;
    let mut shell = Shell::con_config(ANCHO, 1.0, config);
    shell.bloquear("23:50".into(), "jueves, 17 de septiembre".into(), pantalla);
    if let Some(b) = shell.bloqueo_mut() {
        b.medios.push(Medio {
            fuente: None,
            caratula: None,
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Surface of a Glass Lake".into(),
            detalle: "_1_975".into(),
            color: iced_core::Color::from_rgb(0.35, 0.4, 0.9),
            progreso: Some(0.25),
            posicion: Some(13),
            duracion: Some(52),
            reproduciendo: false,
        });
    }
    assert_eq!(
        shell.bloqueo_mut().map(|b| {
            b.estado = Estado::Escribiendo;
        }),
        Some(())
    );

    let (w, h) = shell
        .bloqueo_buffer_size()
        .expect("hay superficie de bloqueo");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf);
    volcar("BOOKOS_BLOQUEO_1280_PNG", &buf, w, h);
}

#[test]
fn ahorro_cambia_el_perfil_sin_cerrar_el_centro() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::centro());
    let accion = shell.emergente_pulsar(212.5, 182.0);
    assert!(matches!(
        accion,
        Some(bookos_shell::Accion::PerfilEnergia(_))
    ));
    assert!(shell.hay_emergente(), "ahorro cerró el centro de control");
    let siguiente = shell.emergente_pulsar(212.5, 182.0);
    assert_ne!(accion, siguiente, "el segundo clic repitió el mismo perfil");
}
