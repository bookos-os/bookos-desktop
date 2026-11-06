use super::*;

use greetd_ipc::codec::SyncCodec;
use iced::Color;
use iced::futures::{StreamExt, executor::block_on};
use iced_runtime::Action;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread::JoinHandle;

struct Servidor {
    directorio: PathBuf,
    hilo: Option<JoinHandle<anyhow::Result<()>>>,
}

impl Servidor {
    // 0.1.1
    fn nuevo(
        atender: impl FnOnce(UnixStream) -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<(Acceso, Self)> {
        Self::con_personas(Vec::new(), atender)
    }

    // 0.1.1
    fn con_personas(
        personas: Vec<Persona>,
        atender: impl FnOnce(UnixStream) -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<(Acceso, Self)> {
        let identificador = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let directorio = std::env::temp_dir().join(format!(
            "bookos-greeter-{}-{identificador}",
            std::process::id()
        ));
        std::fs::create_dir(&directorio)?;
        let ruta = directorio.join("greetd.sock");
        let escucha = UnixListener::bind(&ruta)?;
        let hilo = std::thread::spawn(move || {
            let (socket, _) = escucha.accept()?;
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            atender(socket)
        });
        let (acceso, _) =
            Acceso::nuevo(Some(ruta), "/usr/local/bin/bookos-session".into(), personas);
        Ok((
            acceso,
            Self {
                directorio,
                hilo: Some(hilo),
            },
        ))
    }

    // 0.1.1
    fn terminar(mut self) -> anyhow::Result<()> {
        self.hilo
            .take()
            .expect("servidor pendiente de verificar")
            .join()
            .expect("servidor simulado sin pánicos")
    }
}

impl Drop for Servidor {
    // 0.1.1
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directorio);
    }
}

// 0.1.1
fn recibir(tarea: Task<Mensaje>) -> Mensaje {
    let mut acciones =
        iced_runtime::task::into_stream(tarea).expect("la petición produce una respuesta");
    block_on(async {
        while let Some(accion) = acciones.next().await {
            // Elegir de la lista pide también el fondo; aquí interesa greetd.
            if let Action::Output(mensaje @ Mensaje::Recibido(..)) = accion {
                return mensaje;
            }
        }
        panic!("la petición no termina sin respuesta")
    })
}

// 0.1.1
fn pregunta(socket: &mut UnixStream, tipo: AuthMessageType, texto: &str) -> anyhow::Result<()> {
    Response::AuthMessage {
        auth_message_type: tipo,
        auth_message: texto.into(),
    }
    .write_to(socket)?;
    Ok(())
}

// 0.1.1
fn comprobar_respuesta(socket: &mut UnixStream, esperada: Option<&str>) -> anyhow::Result<()> {
    match Request::read_from(socket)? {
        Request::PostAuthMessageResponse { response } => assert_eq!(response.as_deref(), esperada),
        _ => panic!("PAM recibe una respuesta a su pregunta"),
    }
    Ok(())
}

// 0.1.1
fn comenzar(socket: &mut UnixStream) -> anyhow::Result<Request> {
    assert!(matches!(
        Request::read_from(socket)?,
        Request::CancelSession
    ));
    Response::Success.write_to(socket)?;
    Ok(Request::read_from(socket)?)
}

#[test]
// 0.1.1
fn autentica_con_varias_preguntas_y_sale_solo_tras_arrancar() -> anyhow::Result<()> {
    let (mut acceso, servidor) = Servidor::nuevo(|mut socket| {
        assert!(
            matches!(comenzar(&mut socket)?, Request::CreateSession { username } if username == "eve")
        );
        pregunta(&mut socket, AuthMessageType::Secret, "Contraseña:")?;
        comprobar_respuesta(&mut socket, Some("secreto"))?;
        pregunta(&mut socket, AuthMessageType::Visible, "Código de acceso:")?;
        comprobar_respuesta(&mut socket, Some("123456"))?;
        pregunta(&mut socket, AuthMessageType::Info, "Bienvenida")?;
        comprobar_respuesta(&mut socket, None)?;
        Response::Success.write_to(&mut socket)?;
        assert!(
            matches!(Request::read_from(&mut socket)?, Request::StartSession { cmd, .. } if cmd == ["/usr/local/bin/bookos-session"])
        );
        Response::Success.write_to(&mut socket)?;
        Ok(())
    })?;
    let _ = acceso.actualizar(Mensaje::Usuario(" eve ".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    // Enter repetido mientras PAM responde no puede duplicar la petición.
    assert!(iced_runtime::task::into_stream(acceso.actualizar(Mensaje::Continuar)).is_none());
    let _ = acceso.actualizar(mensaje);
    assert!(matches!(
        acceso.estado,
        Estado::Pregunta {
            tipo: TipoEntrada::Secreta,
            ..
        }
    ));
    let _ = acceso.actualizar(Mensaje::Respuesta("secreto".into()));
    let tarea = acceso.actualizar(Mensaje::Continuar);
    assert!(acceso.respuesta.is_empty());
    let mensaje = recibir(tarea);
    let _ = acceso.actualizar(mensaje);
    assert!(matches!(
        acceso.estado,
        Estado::Pregunta {
            tipo: TipoEntrada::Visible,
            ..
        }
    ));
    let _ = acceso.actualizar(Mensaje::Respuesta("123456".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    // El aviso se contesta solo: pam_fprintd no lee el lector hasta entonces.
    let tarea = acceso.actualizar(mensaje);
    assert!(matches!(acceso.estado, Estado::Aviso(_)));
    let mensaje = recibir(tarea);
    let tarea = acceso.actualizar(mensaje);
    assert!(matches!(acceso.estado, Estado::Ocupado(Operacion::Iniciar)));
    let mensaje = recibir(tarea);
    let tarea = acceso.actualizar(mensaje);
    let mut acciones =
        iced_runtime::task::into_stream(tarea).expect("el login confirmado cierra el greeter");
    assert!(matches!(block_on(acciones.next()), Some(Action::Exit)));
    servidor.terminar()
}

#[test]
// 0.1.1
fn cancela_pam_antes_de_permitir_cambiar_de_usuario() -> anyhow::Result<()> {
    let (mut acceso, servidor) = Servidor::nuevo(|mut socket| {
        assert!(matches!(
            comenzar(&mut socket)?,
            Request::CreateSession { .. }
        ));
        pregunta(&mut socket, AuthMessageType::Secret, "Contraseña:")?;
        assert!(matches!(
            Request::read_from(&mut socket)?,
            Request::CancelSession
        ));
        Response::Success.write_to(&mut socket)?;
        assert!(
            matches!(comenzar(&mut socket)?, Request::CreateSession { username } if username == "otra")
        );
        Response::Error {
            error_type: ErrorType::AuthError,
            description: "Rechazada".into(),
        }
        .write_to(&mut socket)?;
        Ok(())
    })?;
    let _ = acceso.actualizar(Mensaje::Usuario("eve".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    let _ = acceso.actualizar(mensaje);
    let _ = acceso.actualizar(Mensaje::Respuesta("secreto".into()));
    let tarea = acceso.actualizar(Mensaje::Cancelar);
    assert!(acceso.respuesta.is_empty());
    assert!(matches!(
        acceso.estado,
        Estado::Ocupado(Operacion::Cancelar)
    ));
    let mensaje = recibir(tarea);
    let _ = acceso.actualizar(mensaje);
    assert!(matches!(acceso.estado, Estado::Usuario));
    let _ = acceso.actualizar(Mensaje::Usuario("otra".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    let _ = acceso.actualizar(mensaje);
    assert!(matches!(acceso.estado, Estado::Usuario));
    assert!(acceso.error.is_some());
    servidor.terminar()
}

#[test]
// 0.1.1
fn un_fallo_al_arrancar_mantiene_el_greeter_abierto() -> anyhow::Result<()> {
    let (mut acceso, servidor) = Servidor::nuevo(|mut socket| {
        assert!(matches!(
            comenzar(&mut socket)?,
            Request::CreateSession { .. }
        ));
        Response::Success.write_to(&mut socket)?;
        assert!(matches!(
            Request::read_from(&mut socket)?,
            Request::StartSession { .. }
        ));
        Response::Error {
            error_type: ErrorType::Error,
            description: "No se pudo arrancar".into(),
        }
        .write_to(&mut socket)?;
        Ok(())
    })?;
    let _ = acceso.actualizar(Mensaje::Usuario("eve".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    let mensaje = recibir(acceso.actualizar(mensaje));
    let tarea = acceso.actualizar(mensaje);
    assert!(matches!(acceso.estado, Estado::Usuario));
    assert!(
        acceso
            .error
            .as_deref()
            .is_some_and(|texto| texto.contains("No se pudo arrancar"))
    );
    let mut acciones =
        iced_runtime::task::into_stream(tarea).expect("se devuelve el foco al usuario");
    assert!(matches!(block_on(acciones.next()), Some(Action::Widget(_))));
    servidor.terminar()
}

#[test]
// 0.1.1
fn la_vista_previa_no_autentica_y_los_diagnosticos_ocultan_respuestas() {
    let (mut acceso, _) = Acceso::nuevo(None, "/sesion-no-utilizada".into(), Vec::new());
    let _ = acceso.actualizar(Mensaje::Usuario("eve".into()));
    assert!(iced_runtime::task::into_stream(acceso.actualizar(Mensaje::Continuar)).is_none());
    assert!(matches!(acceso.estado, Estado::Usuario));
    assert!(
        acceso
            .error
            .as_deref()
            .is_some_and(|texto| texto.starts_with("Vista previa"))
    );
    assert!(!format!("{:?}", Mensaje::Respuesta("secreto".into())).contains("secreto"));
}

// 0.1.1
fn de_pega(cuentas: &[(&str, &str)]) -> Vec<Persona> {
    cuentas
        .iter()
        .map(|(cuenta, nombre)| Persona::nueva(cuenta, nombre, None, Path::new("/no-existe")))
        .collect()
}

#[test]
// 0.1.1
fn elegir_en_la_lista_abre_la_sesion_pam_de_esa_cuenta() -> anyhow::Result<()> {
    let personas = de_pega(&[("daniel", "Daniel"), ("laura", "Laura")]);
    let (mut acceso, servidor) = Servidor::con_personas(personas, |mut socket| {
        assert!(
            matches!(comenzar(&mut socket)?, Request::CreateSession { username } if username == "laura")
        );
        pregunta(&mut socket, AuthMessageType::Secret, "Contraseña:")?;
        Ok(())
    })?;
    assert!(acceso.en_lista());
    let _ = acceso.actualizar(Mensaje::Mover(1));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    let _ = acceso.actualizar(mensaje);
    assert_eq!(acceso.usuario, "laura");
    assert!(matches!(acceso.estado, Estado::Pregunta { .. }));
    servidor.terminar()
}

#[test]
// 0.1.1
fn una_contrasena_mala_vuelve_a_preguntar_sin_salir_del_usuario() -> anyhow::Result<()> {
    let (mut acceso, servidor) = Servidor::nuevo(|mut socket| {
        for _ in 0..2 {
            assert!(
                matches!(comenzar(&mut socket)?, Request::CreateSession { username } if username == "eve")
            );
            pregunta(&mut socket, AuthMessageType::Secret, "Contraseña:")?;
            comprobar_respuesta(&mut socket, Some("mala"))?;
            Response::Error {
                error_type: ErrorType::AuthError,
                description: "Rechazada".into(),
            }
            .write_to(&mut socket)?;
        }
        // La segunda vez se rechaza la cuenta sin preguntar: ahí ya no se insiste.
        assert!(matches!(
            comenzar(&mut socket)?,
            Request::CreateSession { .. }
        ));
        Response::Error {
            error_type: ErrorType::AuthError,
            description: "Bloqueada".into(),
        }
        .write_to(&mut socket)?;
        Ok(())
    })?;
    let _ = acceso.actualizar(Mensaje::Usuario("eve".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    let _ = acceso.actualizar(mensaje);
    for _ in 0..2 {
        let _ = acceso.actualizar(Mensaje::Respuesta("mala".into()));
        let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
        let reintento = acceso.actualizar(mensaje);
        assert!(acceso.error.is_some());
        assert!(matches!(
            acceso.estado,
            Estado::Ocupado(Operacion::Autenticar)
        ));
        let mensaje = recibir(reintento);
        let _ = acceso.actualizar(mensaje);
    }
    assert!(matches!(acceso.estado, Estado::Usuario));
    assert!(acceso.error.is_some());
    servidor.terminar()
}

#[test]
// añadido en la 0.2.0
fn la_huella_suelta_la_pregunta_de_pam_bookos_huella() -> anyhow::Result<()> {
    let (mut acceso, servidor) = Servidor::nuevo(|mut socket| {
        assert!(matches!(
            comenzar(&mut socket)?,
            Request::CreateSession { .. }
        ));
        pregunta(&mut socket, AuthMessageType::Secret, PREGUNTA_HUELLA)?;
        // Con la huella casada la respuesta es vacía: solo suelta la
        // conversación, el módulo ya sabe que casó.
        comprobar_respuesta(&mut socket, Some(""))?;
        Response::Success.write_to(&mut socket)?;
        assert!(matches!(
            Request::read_from(&mut socket)?,
            Request::StartSession { .. }
        ));
        Response::Success.write_to(&mut socket)?;
        Ok(())
    })?;
    let _ = acceso.actualizar(Mensaje::Usuario("eve".into()));
    let mensaje = recibir(acceso.actualizar(Mensaje::Continuar));
    let _ = acceso.actualizar(mensaje);
    assert!(matches!(&acceso.estado, Estado::Pregunta { texto, .. } if texto == PREGUNTA_HUELLA));
    // Un fallo del lector avisa sin contestar: la contraseña sigue valiendo.
    let _ = acceso.actualizar(Mensaje::Huella("no-coincide 1".into()));
    assert!(acceso.error.is_some());
    assert!(matches!(acceso.estado, Estado::Pregunta { .. }));
    let mensaje = recibir(acceso.actualizar(Mensaje::Huella("coincide".into())));
    let inicio = acceso.actualizar(mensaje);
    let _ = acceso.actualizar(recibir(inicio));
    servidor.terminar()
}

#[test]
// 0.1.1
fn solo_se_listan_las_cuentas_de_persona() {
    let passwd = "root:x:0:0:Super User:/root:/bin/bash\n\
        sddm:x:977:977:SDDM:/var/lib/sddm:/usr/sbin/nologin\n\
        eve:x:1000:1000:Evelyn,,,:/home/eve:/bin/bash\n\
        invitada:x:1001:1001::/home/invitada:/usr/bin/zsh\n\
        copia:x:1002:1002:Copias:/srv/copia:/sbin/nologin\n\
        nobody:x:65534:65534:Kernel Overflow User:/:/usr/sbin/nologin\n";
    assert_eq!(
        usuarios::listables(passwd),
        [
            ("eve".to_string(), 1000, "Evelyn".to_string()),
            ("invitada".to_string(), 1001, "invitada".to_string()),
        ]
    );
}

#[test]
// 0.1.1
fn lo_publicado_por_otro_usuario_no_se_acepta() -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let carpeta = std::env::temp_dir().join(format!("bookos-greeter-pub-{}", std::process::id()));
    std::fs::create_dir_all(&carpeta)?;
    let archivo = carpeta.join("panel.conf");
    std::fs::write(&archivo, "tema = claro\n")?;
    let yo = std::fs::metadata(&archivo)?.uid();
    let enlace = carpeta.join("enlace");
    std::os::unix::fs::symlink(&archivo, &enlace)?;
    assert!(usuarios::publicado_valido(&archivo, yo, 1024).is_some());
    assert!(usuarios::publicado_valido(&archivo, yo + 1, 1024).is_none());
    assert!(usuarios::publicado_valido(&archivo, yo, 4).is_none());
    assert!(usuarios::publicado_valido(&enlace, yo, 1024).is_none());
    std::fs::remove_dir_all(&carpeta)?;
    Ok(())
}

#[test]
// 0.1.1
fn la_apariencia_publicada_viste_a_su_usuario() -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let raiz = std::env::temp_dir().join(format!("bookos-greeter-ap-{}", std::process::id()));
    let carpeta = raiz.join("laura");
    std::fs::create_dir_all(&carpeta)?;
    std::fs::write(
        carpeta.join("panel.conf"),
        "tema = claro\nacento = rojo\nbloqueo_reloj_fuente = mono\n",
    )?;
    std::fs::write(carpeta.join("fondo-claro"), b"no es una imagen")?;
    let yo = std::fs::metadata(&carpeta)?.uid();
    let propia = Persona::nueva("laura", "Laura", Some(yo), &raiz);
    assert_eq!(propia.apariencia.tema, bookos_shell::tema::Tema::Claro);
    assert_eq!(propia.apariencia.acento, bookos_shell::tema::Acento::Rojo);
    assert_eq!(
        propia.apariencia.bloqueo.reloj_fuente,
        bookos_shell::FuenteReloj::Mono
    );
    assert_eq!(propia.apariencia.fondo, Some(carpeta.join("fondo-claro")));
    // La misma carpeta a nombre de otro uid es la de alguien que se hace pasar
    // por Laura: se ignora entera.
    let ajena = Persona::nueva("laura", "Laura", Some(yo + 1), &raiz);
    assert_eq!(ajena.apariencia.tema, bookos_shell::tema::Tema::Oscuro);
    assert!(ajena.apariencia.fondo.is_none());
    std::fs::remove_dir_all(&raiz)?;
    Ok(())
}

#[test]
// 0.1.1
fn un_clic_sobre_una_fila_elige_esa_cuenta() {
    use iced::mouse::{self, Cursor};
    use iced::{Event, Font, Pixels, Point, Size};
    use iced_runtime::user_interface::{Cache, UserInterface};

    let personas = de_pega(&[("daniel", "Daniel"), ("laura", "Laura")]);
    let (mut acceso, _) = Acceso::nuevo(None, "/sesion-no-utilizada".into(), personas);
    acceso.pantalla = Size::new(1280.0, 800.0);
    let (_, k, arriba) = acceso.lista();
    // El centro del avatar de la segunda fila, con la lista en reposo.
    let punto = Point::new(
        (1280.0 - vista::ANCHO_LISTA * k) / 2.0 + 20.0,
        arriba + (lente::ALTO + lente::HUECO) * k + lente::ALTO * k / 2.0,
    );
    let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, Pixels(16.0));
    let mut interfaz = UserInterface::build(
        acceso.vista(),
        acceso.pantalla,
        Cache::default(),
        &mut renderer,
    );
    let mut mensajes = Vec::new();
    let _ = interfaz.update(
        &[
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ],
        Cursor::Available(punto),
        &mut renderer,
        &mut iced_runtime::core::clipboard::Null,
        &mut mensajes,
    );
    assert!(
        matches!(mensajes[..], [Mensaje::Elegir(1)]),
        "mensajes: {mensajes:?}"
    );
    drop(interfaz);
    let _ = acceso.actualizar(Mensaje::Elegir(1));
    assert!(!acceso.en_lista());
    assert_eq!(acceso.usuario, "laura");
}

#[test]
// 0.1.1
fn renderiza_las_vistas_para_inspeccion() -> anyhow::Result<()> {
    use bookos_shell::tema::Tema;
    use iced::mouse::Cursor;
    use iced::{Font, Pixels, Rectangle, Size};
    use iced_graphics::Viewport;
    use iced_runtime::core::renderer::Style;
    use iced_runtime::user_interface::{Cache, UserInterface};

    let Some(directorio) = std::env::var_os("BOOKOS_GREETER_PREVIEW_DIR") else {
        return Ok(());
    };
    let directorio = PathBuf::from(directorio);
    std::fs::create_dir_all(&directorio)?;
    // La escala que pone BookOS en este panel: la que el greeter recibe ahora publicada.
    let (ancho, alto, escala) = (2880, 1800, 2.0);
    let publicada = std::env::var_os("BOOKOS_GREETER_PREVIEW_PUBLICADO").map(PathBuf::from);
    let pregunta = || Estado::Pregunta {
        texto: "Contraseña:".into(),
        tipo: TipoEntrada::Secreta,
    };
    for (nombre, tema_de, preparar) in [
        ("lista", Tema::Oscuro, (|_| {}) as fn(&mut Acceso)),
        ("lente", Tema::Oscuro, |a| {
            a.foco = Foco::Fila(1);
            a.fuerza.fijar(1.0);
        }),
        ("lente-claro", Tema::Claro, |a| {
            a.foco = Foco::Fila(1);
            a.fuerza.fijar(1.0);
        }),
        ("acceso", Tema::Oscuro, |a| a.usuario = "laura".into()),
        ("fallo", Tema::Oscuro, |a| {
            a.usuario = "laura".into();
            a.error = Some("No se ha podido autenticar. Comprueba tus datos.".into());
        }),
        ("menu", Tema::Oscuro, |a| a.menu.fijar(1.0)),
        ("otro", Tema::Claro, |a| a.manual = true),
    ] {
        let personas = de_pega(&[
            ("daniel", "Daniel"),
            ("laura", "Laura"),
            ("alvaro", "Álvaro de la Fuente y Larguísimo Apellido"),
        ]);
        let (mut acceso, _) = Acceso::nuevo(None, "/sesion-no-utilizada".into(), personas);
        bookos_shell::tema::aplicar(tema_de);
        acceso.pantalla = Size::new(ancho as f32, alto as f32);
        acceso.escala = escala;
        acceso.cambio_desde -= std::time::Duration::from_secs(5);
        // Con BOOKOS_GREETER_PREVIEW_PUBLICADO se viste como una carpeta
        // publicada de verdad: fondo, tema y paleta de esa sesión.
        if let Some(publicada) = &publicada {
            use std::os::unix::fs::MetadataExt;
            let uid = std::fs::metadata(publicada)?.uid();
            let cuenta = publicada
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("eve");
            let raiz = publicada.parent().unwrap_or(Path::new("/"));
            acceso.personas[0] = Persona::nueva(cuenta, "Daniel", Some(uid), raiz);
            acceso.personas[0].cuenta = "daniel".into();
            acceso.vestir(0);
        }
        let ruta_fondo = acceso.personas[0]
            .apariencia
            .fondo
            .clone()
            .or_else(|| usuarios::fondo_por_defecto(tema_de));
        let cargar = |densidad| {
            ruta_fondo
                .as_deref()
                .and_then(|ruta| usuarios::cargar_fondo(ruta, (ancho, alto), densidad, false))
        };
        acceso.fondo = cargar(1.0);
        acceso.teclado = Some("ES".into());
        preparar(&mut acceso);
        if !acceso.usuario.is_empty() {
            acceso.estado = pregunta();
            acceso.respuesta = "Prueba123".into();
        }
        let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, Pixels(16.0));
        let viewport = Viewport::with_physical_size(Size::new(ancho, alto), 1.0);
        let mut pixmap = tiny_skia::Pixmap::new(ancho, alto)
            .expect("dimensiones positivas para la vista previa");
        let mut mascara = tiny_skia::Mask::new(ancho, alto).expect("máscara del tamaño del pixmap");
        // Dos pasadas: la primera paga rasterizar SVG y subir imágenes a la
        // caché; la segunda es lo que cuesta un fotograma de animación.
        for pasada in 0..3 {
            // La tercera repinta solo la zona de la lista, que es lo que hace
            // el compositor de iced con el daño de un fotograma de la lente.
            let zona = if pasada == 2 {
                Rectangle::new(iced::Point::new(1100.0, 1100.0), Size::new(700.0, 650.0))
            } else {
                Rectangle::with_size(viewport.logical_size())
            };
            let inicio = std::time::Instant::now();
            let mut interfaz = UserInterface::build(
                acceso.vista(),
                viewport.logical_size(),
                Cache::default(),
                &mut renderer,
            );
            let _ = interfaz.update(
                &[iced::Event::Window(iced::window::Event::RedrawRequested(
                    std::time::Instant::now(),
                ))],
                Cursor::Unavailable,
                &mut renderer,
                &mut iced_runtime::core::clipboard::Null,
                &mut Vec::new(),
            );
            interfaz.draw(
                &mut renderer,
                &crate::tema_iced(&acceso),
                &Style {
                    text_color: Color::WHITE,
                },
                Cursor::Unavailable,
            );
            let interfaz_en = inicio.elapsed();
            renderer.draw(
                &mut pixmap.as_mut(),
                &mut mascara,
                &viewport,
                &[zona],
                vista::FONDO,
            );
            println!(
                "{nombre} pasada {pasada}: interfaz {interfaz_en:?}, total {:?}",
                inicio.elapsed()
            );
        }
        for pixel in pixmap.data_mut().as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        // Las losas tienen que dar el mismo fondo que la imagen en una pieza:
        // una fila y una columna que las cruzan todas delatan una costura o
        // una losa corrida un píxel.
        if let Some(usuarios::Fondo { losas }) = cargar(64.0)
            && let iced::widget::image::Handle::Rgba { pixels, .. } = &losas[0][0].0
        {
            let en = |x: u32, y: u32| ((y * ancho + x) * 4) as usize;
            for (x, y) in (0..ancho).map(|x| (x, 3)).chain((0..alto).map(|y| (3, y))) {
                assert_eq!(
                    pixmap.data()[en(x, y)..en(x, y) + 3],
                    pixels[en(x, y)..en(x, y) + 3],
                    "{nombre}: el fondo no coincide en ({x}, {y})"
                );
            }
        }
        pixmap.save_png(directorio.join(format!("{nombre}.png")))?;
    }
    Ok(())
}

#[test]
// 0.1.1
fn cambia_idioma_teclado_y_visibilidad_de_contrasena() {
    let (mut acceso, _) = Acceso::nuevo(None, "/sesion-no-utilizada".into(), Vec::new());
    let _ = acceso.actualizar(Mensaje::AbrirPanel(Some(Panel::Idioma)));
    assert_eq!(acceso.panel_abierto, Some(Panel::Idioma));
    assert!(acceso.menu.destino() > 0.0);

    let _ = acceso.actualizar(Mensaje::ElegirIdioma(Idioma::En));
    assert_eq!(acceso.idioma, Idioma::En);

    let _ = acceso.actualizar(Mensaje::ElegirTeclado("us".into()));
    assert_eq!(acceso.teclado.as_deref(), Some("US"));
    assert_eq!(acceso.teclado_codigo, "us");

    // Alt+Shift avanza a la siguiente distribución de la lista.
    let _ = acceso.actualizar(Mensaje::SiguienteTeclado);
    assert_eq!(acceso.teclado_codigo, "gb");

    // Alternar visibilidad de contraseña y Bloq Mayús.
    assert!(!acceso.mostrar_respuesta);
    let _ = acceso.actualizar(Mensaje::AlternarMostrarClave);
    assert!(acceso.mostrar_respuesta);
    let _ = acceso.actualizar(Mensaje::AlternarMayusculas);
    assert!(acceso.caps_lock);

    // Esc con un panel abierto cierra el panel sin cancelar la sesión.
    let _ = acceso.actualizar(Mensaje::Cancelar);
    assert_eq!(acceso.panel_abierto, None);
    assert_eq!(acceso.menu.destino(), 0.0);
}

#[test]
// 0.1.1
fn gestiona_redes_wifi_y_perfiles_de_energia_en_vista_previa() {
    let (mut acceso, _) = Acceso::nuevo(None, "/sesion-no-utilizada".into(), Vec::new());
    let _ = acceso.actualizar(Mensaje::AbrirPanel(Some(Panel::Red)));
    assert_eq!(acceso.panel_abierto, Some(Panel::Red));
    assert_eq!(acceso.wifi_estado.ssid_actual.as_deref(), Some("BookOS-5G"));

    // Elegir una red segura no guardada pide contraseña antes de conectar.
    let _ = acceso.actualizar(Mensaje::WifiElegir(1));
    assert_eq!(acceso.wifi_conectar_a.as_deref(), Some("Invitados"));
    let _ = acceso.actualizar(Mensaje::WifiClave("clave1234".into()));
    let _ = acceso.actualizar(Mensaje::WifiConectar);
    assert_eq!(acceso.wifi_conectar_a, None);
    assert_eq!(acceso.wifi_estado.ssid_actual.as_deref(), Some("Invitados"));

    // Desconectar y apagar Wi-Fi.
    let _ = acceso.actualizar(Mensaje::WifiDesconectar);
    assert_eq!(acceso.wifi_estado.ssid_actual, None);
    let _ = acceso.actualizar(Mensaje::WifiAlternar(false));
    assert!(!acceso.wifi_estado.encendida);

    // Cambiar el modo de energía en el panel de batería.
    let _ = acceso.actualizar(Mensaje::AbrirPanel(Some(Panel::Bateria)));
    assert_eq!(acceso.panel_abierto, Some(Panel::Bateria));
    let _ = acceso.actualizar(Mensaje::PerfilEnergia("performance".into()));
    assert_eq!(acceso.bateria_detalle.perfil, "performance");
}

