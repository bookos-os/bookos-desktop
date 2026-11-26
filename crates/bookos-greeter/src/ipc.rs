use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{Request, Response};
use iced::futures::channel::oneshot;

type Trabajo = (
    Request,
    crate::sistema::Idioma,
    oneshot::Sender<Result<Response, String>>,
);

pub struct Cliente {
    envio: mpsc::Sender<Trabajo>,
}

impl Cliente {
    // 0.1.1
    pub fn nuevo(ruta: PathBuf) -> Self {
        let (envio, recepcion) = mpsc::channel::<Trabajo>();
        // PAM puede esperar por hardware o aplicar una demora tras un fallo.
        // El socket pertenece a este hilo para que nunca bloquee la interfaz.
        std::thread::spawn(move || {
            let mut socket = None;
            for (peticion, idioma, respuesta) in recepcion {
                let resultado = intercambiar(&ruta, &mut socket, &peticion, idioma);
                if resultado.is_err() {
                    socket = None;
                }
                let _ = respuesta.send(resultado.map_err(|error| format!("{error:#}")));
            }
        });
        Self { envio }
    }

    // 0.1.1
    pub fn enviar(
        &self,
        peticion: Request,
        idioma: crate::sistema::Idioma,
    ) -> impl Future<Output = Result<Response, String>> + use<> {
        let (envio, recepcion) = oneshot::channel();
        let enviado = self.envio.send((peticion, idioma, envio)).is_ok();
        async move {
            if !enviado {
                return Err(idioma
                    .t(
                        "El hilo de comunicación con greetd ha terminado.",
                        "The greetd communication thread has stopped.",
                    )
                    .into());
            }
            recepcion.await.map_err(|_| {
                idioma
                    .t(
                        "Se ha perdido la comunicación con greetd.",
                        "Lost connection to greetd.",
                    )
                    .to_string()
            })?
        }
    }
}

// 0.1.1
fn intercambiar(
    ruta: &std::path::Path,
    socket: &mut Option<UnixStream>,
    peticion: &Request,
    idioma: crate::sistema::Idioma,
) -> anyhow::Result<Response> {
    if socket.is_none() {
        let nuevo = UnixStream::connect(ruta).with_context(|| {
            idioma.t(
                "No se puede conectar con greetd",
                "Cannot connect to greetd",
            )
        })?;
        nuevo.set_read_timeout(Some(Duration::from_secs(120)))?;
        nuevo.set_write_timeout(Some(Duration::from_secs(10)))?;
        *socket = Some(nuevo);
    }
    let socket = socket
        .as_mut()
        .expect("socket conectado antes de intercambiar");
    if matches!(peticion, Request::CreateSession { .. }) {
        // greetd conserva la conversación si un cliente cierra el socket sin
        // cancelar. Un reintento debe empezar sin una sesión PAM pendiente.
        Request::CancelSession.write_to(socket)?;
        anyhow::ensure!(
            matches!(Response::read_from(socket)?, Response::Success),
            "{}",
            idioma.t(
                "greetd no pudo cancelar la autenticación anterior",
                "greetd couldn't cancel the previous authentication",
            )
        );
    }
    peticion.write_to(socket).with_context(|| {
        idioma.t(
            "No se pudo enviar la petición a greetd",
            "Couldn't send request to greetd",
        )
    })?;
    Response::read_from(socket).with_context(|| {
        idioma.t(
            "No se pudo recibir la respuesta de greetd",
            "Couldn't receive response from greetd",
        )
    })
}

// 0.1.1
pub fn iniciar_sesion(programa: &str, teclado: &str, locale: &str) -> Request {
    Request::StartSession {
        cmd: vec![programa.into()],
        env: vec![
            "XDG_SESSION_TYPE=wayland".into(),
            "XDG_CURRENT_DESKTOP=BookOS".into(),
            "XDG_SESSION_DESKTOP=BookOS".into(),
            format!("XKB_DEFAULT_LAYOUT={teclado}"),
            format!("LANG={locale}"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greetd_ipc::{AuthMessageType, ErrorType};

    #[test]
    fn conserva_el_socket_durante_pam_y_el_arranque() -> anyhow::Result<()> {
        let (cliente, mut servidor) = UnixStream::pair()?;
        let hilo = std::thread::spawn(move || -> anyhow::Result<()> {
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::CancelSession
            ));
            Response::Success.write_to(&mut servidor)?;
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::CreateSession { username } if username == "eve"
            ));
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Contraseña:".into(),
            }
            .write_to(&mut servidor)?;
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::PostAuthMessageResponse { response: Some(valor) } if valor == "prueba"
            ));
            Response::Success.write_to(&mut servidor)?;
            match Request::read_from(&mut servidor)? {
                Request::StartSession { cmd, env } => {
                    assert_eq!(cmd, ["/usr/local/bin/bookos-session"]);
                    assert!(env.contains(&"XDG_SESSION_TYPE=wayland".into()));
                    assert!(env.contains(&"XKB_DEFAULT_LAYOUT=es".into()));
                    assert!(env.contains(&"LANG=es_ES.UTF-8".into()));
                    assert!(
                        !env.iter()
                            .any(|valor| valor.starts_with("WAYLAND_DISPLAY="))
                    );
                }
                _ => panic!("solo se arranca una sesión autenticada"),
            }
            Response::Success.write_to(&mut servidor)?;
            Ok(())
        });
        let mut socket = Some(cliente);
        let ruta = std::path::Path::new("/socket-no-utilizado");
        assert!(matches!(
            intercambiar(
                ruta,
                &mut socket,
                &Request::CreateSession {
                    username: "eve".into()
                },
                crate::sistema::Idioma::Es,
            )?,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                ..
            }
        ));
        assert!(matches!(
            intercambiar(
                ruta,
                &mut socket,
                &Request::PostAuthMessageResponse {
                    response: Some("prueba".into())
                },
                crate::sistema::Idioma::Es,
            )?,
            Response::Success
        ));
        assert!(matches!(
            intercambiar(
                ruta,
                &mut socket,
                &iniciar_sesion("/usr/local/bin/bookos-session", "es", "es_ES.UTF-8"),
                crate::sistema::Idioma::Es,
            )?,
            Response::Success
        ));
        hilo.join().expect("el servidor simulado no falla")?;
        Ok(())
    }

    #[test]
    fn permite_reintentar_tras_un_error_de_autenticacion() -> anyhow::Result<()> {
        let (cliente, mut servidor) = UnixStream::pair()?;
        let hilo = std::thread::spawn(move || -> anyhow::Result<()> {
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::CancelSession
            ));
            Response::Success.write_to(&mut servidor)?;
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::CreateSession { .. }
            ));
            Response::Error {
                error_type: ErrorType::AuthError,
                description: "Cuenta rechazada".into(),
            }
            .write_to(&mut servidor)?;
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::CancelSession
            ));
            Response::Success.write_to(&mut servidor)?;
            assert!(matches!(
                Request::read_from(&mut servidor)?,
                Request::CreateSession { .. }
            ));
            Response::Success.write_to(&mut servidor)?;
            Ok(())
        });
        let mut socket = Some(cliente);
        let ruta = std::path::Path::new("/socket-no-utilizado");
        let peticion = Request::CreateSession {
            username: "eve".into(),
        };
        assert!(matches!(
            intercambiar(ruta, &mut socket, &peticion, crate::sistema::Idioma::Es)?,
            Response::Error { .. }
        ));
        assert!(matches!(
            intercambiar(ruta, &mut socket, &peticion, crate::sistema::Idioma::Es)?,
            Response::Success
        ));
        hilo.join().expect("el servidor simulado no falla")?;
        Ok(())
    }
}
