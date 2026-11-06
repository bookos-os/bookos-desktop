//! Las teclas de función del portátil: volumen, brillo y el touchpad.
//!
//! Todas comparten forma: el compositor las intercepta antes que nadie —han de
//! funcionar aunque el foco lo tenga una aplicación a pantalla completa—, hace
//! el cambio y enseña el aviso con el nivel que ha quedado.
//!
//! **Por qué los keysyms van a mano.** Son los `XF86` de siempre y sus valores
//! están fijados desde hace treinta años; escribirlos aquí es más estable que
//! depender de cómo los llame la versión de turno de la biblioteca de teclado,
//! que ya ha cambiado de nombre entre versiones.
//!
//! **Qué no está.** Duplicar la pantalla necesita más de una salida, y el
//! compositor todavía usa solo la primera: mientras eso no exista, la tecla no
//! tendría a qué aplicarse.

use crate::state::BookosComp;

pub const XF86_BAJAR_VOLUMEN: u32 = 0x1008FF11;
pub const XF86_SILENCIAR: u32 = 0x1008FF12;
pub const XF86_SUBIR_VOLUMEN: u32 = 0x1008FF13;
pub const XF86_BRILLO_MAS: u32 = 0x1008FF02;
pub const XF86_BRILLO_MENOS: u32 = 0x1008FF03;
pub const XF86_TECLADO_MAS: u32 = 0x1008FF05;
pub const XF86_TECLADO_MENOS: u32 = 0x1008FF06;
pub const XF86_TECLADO_ALTERNAR: u32 = 0x1008FF04;
pub const XF86_TOUCHPAD: u32 = 0x1008FFA9;
pub const XF86_SILENCIAR_MICRO: u32 = 0x1008FFB2;
pub const XF86_WEBCAM: u32 = 0x1008FF8F;
pub const XF86_FN_ESC: u32 = 0x100811D1;
pub const XF86_CAMARA_ENABLE: u32 = 0x1008124B;
pub const XF86_CAMARA_DISABLE: u32 = 0x1008124C;
pub const XF86_CAMARA_TOGGLE: u32 = 0x1008124D;

/// Lo que hace cada tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tecla {
    /// Hacia dónde: 1 sube, −1 baja. Cuánto lo dice `panel.conf`.
    Volumen(i32),
    Silenciar,
    SilenciarMicro,
    /// Igual que `Volumen`.
    Brillo(i32),
    BrilloTeclado(i32),
    AlternarBrilloTeclado,
    Touchpad,
    Camara(Option<bool>),
    FnLock,
}

// 0.1.0
pub fn resolver(sym: u32) -> Option<Tecla> {
    Some(match sym {
        XF86_SUBIR_VOLUMEN => Tecla::Volumen(1),
        XF86_BAJAR_VOLUMEN => Tecla::Volumen(-1),
        XF86_SILENCIAR => Tecla::Silenciar,
        XF86_SILENCIAR_MICRO => Tecla::SilenciarMicro,
        XF86_WEBCAM | XF86_CAMARA_TOGGLE => Tecla::Camara(None),
        XF86_CAMARA_ENABLE => Tecla::Camara(Some(true)),
        XF86_CAMARA_DISABLE => Tecla::Camara(Some(false)),
        XF86_FN_ESC => Tecla::FnLock,
        XF86_BRILLO_MAS => Tecla::Brillo(1),
        XF86_BRILLO_MENOS => Tecla::Brillo(-1),
        XF86_TECLADO_MAS => Tecla::BrilloTeclado(1),
        XF86_TECLADO_MENOS => Tecla::BrilloTeclado(-1),
        XF86_TECLADO_ALTERNAR => Tecla::AlternarBrilloTeclado,
        XF86_TOUCHPAD => Tecla::Touchpad,
        _ => return None,
    })
}

// 0.1.0
pub fn ejecutar(state: &mut BookosComp, tecla: Tecla) {
    let osd = match tecla {
        Tecla::Volumen(sentido) => {
            volumen(sentido * i32::from(state.pasos.volumen));
            None
        }
        Tecla::Silenciar => {
            silenciar("@DEFAULT_AUDIO_SINK@");
            None
        }
        Tecla::SilenciarMicro => {
            silenciar("@DEFAULT_AUDIO_SOURCE@");
            None
        }
        Tecla::Brillo(sentido) => Some(brillo(state, sentido * i32::from(state.pasos.brillo))),
        Tecla::BrilloTeclado(paso) => Some(brillo_teclado(state, paso)),
        Tecla::AlternarBrilloTeclado => Some(alternar_brillo_teclado(state)),
        Tecla::Touchpad => Some(touchpad(state)),
        Tecla::Camara(estado) => Some(camara(estado)),
        Tecla::FnLock => Some(("teclado", None, Some("Fn lock".into()))),
    };
    if let Some(osd) = osd {
        avisar(state, osd);
    }
    state.needs_redraw = true;
}

/// Sigue subiendo o bajando volumen o brillo mientras se mantenga la tecla.
///
/// El compositor se queda estas teclas, así que nadie las repetía: mantener
/// pulsado daba un solo paso. Empieza tras el retardo de repetición del
/// teclado, como cualquier tecla, y la corta la siguiente pulsación o suelta
/// (`input::tecla`).
// 0.1.0
pub fn repetir_al_mantener(state: &mut BookosComp, tecla: Tecla) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    /// Diez repeticiones por segundo y no el ritmo del teclado (25 por
    /// defecto): con él, el paso de mantener tendría que ser de décimas para
    /// que diera tiempo a soltar donde se quiere.
    const CADA: std::time::Duration = std::time::Duration::from_millis(100);
    if !matches!(tecla, Tecla::Volumen(_) | Tecla::Brillo(_)) {
        return;
    }
    let espera = std::time::Duration::from_millis(state.teclado_repeticion.0.into());
    let token =
        state
            .loop_handle
            .insert_source(Timer::from_duration(espera), move |_, _, state| {
                match tecla {
                    Tecla::Volumen(sentido) => {
                        volumen(sentido * i32::from(state.pasos.volumen_mantener));
                    }
                    Tecla::Brillo(sentido) => {
                        let osd = brillo(state, sentido * i32::from(state.pasos.brillo_mantener));
                        avisar(state, osd);
                    }
                    _ => return TimeoutAction::Drop,
                }
                TimeoutAction::ToDuration(CADA)
            });
    match token {
        Ok(token) => state.repeticion_multimedia = Some(token),
        Err(err) => tracing::warn!("sin repetición de volumen y brillo: {err}"),
    }
}

/// Enseña el aviso y programa su salida.
// 0.1.0
pub(crate) fn avisar(
    state: &mut BookosComp,
    (icono, nivel, texto): (&'static str, Option<u8>, Option<String>),
) {
    if let Some(shell) = state.shell.as_mut() {
        shell.mostrar_osd(icono, nivel, texto);
        despertar_para_la_salida(state);
    }
    state.needs_redraw = true;
}

/// El nombre con que el driver `samsung-galaxybook` registra la tapa de la
/// cámara como dispositivo de entrada.
const TAPA_CAMARA: &str = "Samsung Galaxy Book Camera Lens Cover";

/// Enseña el aviso de la cámara cuando se bloquea o desbloquea con su tecla.
///
/// En el Galaxy Book esa tecla (Fn+F11 en el Book5 Pro) no llega como tecla:
/// el driver la atiende y manda un **interruptor**, `SW_CAMERA_LENS_COVER`
/// —medido con `hexdump`: tipo 5, código 9, valor 1 al bloquearla y 0 al
/// soltarla—. libinput solo entiende los interruptores de tapa y de modo
/// tableta, así que se lee el dispositivo directamente.
///
/// Se abre por su ruta y **no** por libseat: libinput ya lo tiene abierto por
/// ese camino y logind no entrega dos veces el mismo dispositivo. El permiso lo
/// da la regla de udev `session/70-bookos-camara.rules`; sin ella se avisa en
/// el registro y la cámara sigue funcionando, solo que sin aviso.
// 0.1.0
pub fn vigilar_tapa_camara(state: &mut BookosComp) {
    use smithay::reexports::calloop::generic::Generic;
    use smithay::reexports::calloop::{Interest, Mode, PostAction};
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let Some(nodo) = buscar_evento(TAPA_CAMARA) else {
        return;
    };
    let fichero = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(smithay::reexports::rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(&nodo)
    {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(
                ?nodo,
                "sin aviso de la cámara (¿falta la regla de udev?): {err}"
            );
            return;
        }
    };
    let registrado = state.loop_handle.insert_source(
        Generic::new(fichero, Interest::READ, Mode::Level),
        |_, fichero, state| {
            let mut buf = [0u8; EVENTO * 16];
            loop {
                let n = match (&**fichero).read(&mut buf) {
                    Ok(0) => return Ok(PostAction::Remove),
                    Ok(n) => n,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        return Ok(PostAction::Continue);
                    }
                    // Desenchufado o revocado: se deja de escuchar, sin tumbar
                    // el bucle.
                    Err(err) => {
                        tracing::warn!("se dejó de oír la tapa de la cámara: {err}");
                        return Ok(PostAction::Remove);
                    }
                };
                for evento in buf[..n].as_chunks::<EVENTO>().0 {
                    if let Some(activa) = camara_activa(evento) {
                        avisar(state, camara(Some(activa)));
                    }
                }
            }
        },
    );
    if let Err(err) = registrado {
        tracing::warn!("sin aviso de la cámara: {err}");
    }
}

/// Lo que mide un `struct input_event` en 64 bits: 16 bytes de tiempo, y
/// después tipo, código y valor.
const EVENTO: usize = 24;

/// Si el evento es el interruptor de la tapa, dice si la cámara queda activa.
// 0.1.0
fn camara_activa(evento: &[u8; EVENTO]) -> Option<bool> {
    const EV_SW: u16 = 5;
    const SW_CAMERA_LENS_COVER: u16 = 9;
    let tipo = u16::from_ne_bytes([evento[16], evento[17]]);
    let codigo = u16::from_ne_bytes([evento[18], evento[19]]);
    let valor = i32::from_ne_bytes([evento[20], evento[21], evento[22], evento[23]]);
    // Tapa puesta (1) es cámara desactivada.
    (tipo == EV_SW && codigo == SW_CAMERA_LENS_COVER).then_some(valor == 0)
}

/// El `/dev/input/eventN` del dispositivo de entrada con ese nombre.
// 0.1.0
fn buscar_evento(nombre: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/input")
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .find(|e| {
            std::fs::read_to_string(e.path().join("device/name")).is_ok_and(|n| n.trim() == nombre)
        })
        .map(|e| std::path::Path::new("/dev/input").join(e.file_name()))
}

/// Enseña el aviso de la luz del teclado cuando la cambia el **firmware**.
///
/// En el Galaxy Book, Fn+F9 no llega como tecla: lo atiende el driver
/// `samsung-galaxybook`, cambia el LED él solo y avisa por
/// `brightness_hw_changed`, que da «No hay datos» hasta el primer cambio. Sin
/// esto la luz cambiaba sin ningún aviso en pantalla.
///
/// Un hilo que duerme en `poll(POLLPRI)`, que es como sysfs notifica un
/// atributo, y no un temporizador que relea el fichero: no despierta la CPU
/// hasta que alguien pulsa la tecla.
///
/// El mismo `poll` escucha los uevents de `leds`: si el driver se recarga
/// (`modprobe -r` al reinstalar el módulo), el LED nace de nuevo y el fichero
/// abierto queda apuntando al nodo viejo, que ya no notifica nunca. Pasó así:
/// tras una recarga a las 05:26 el hilo seguía dormido en el nodo muerto y
/// Fn+F9 dejó de enseñar el aviso.
#[cfg(feature = "udev")]
// 0.1.0
// modified 0.1.1 -- reabre la luz del teclado si se recarga el driver
pub fn vigilar_luz_teclado(state: &mut BookosComp) {
    use smithay::reexports::calloop::channel;
    let Some((dispositivo, _, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return;
    };
    let ruta = std::path::Path::new("/sys/class/leds")
        .join(&dispositivo)
        .join("brightness_hw_changed");
    // Solo lo tienen los LED que el hardware puede cambiar por su cuenta.
    let Ok(fichero) = std::fs::File::open(&ruta) else {
        return;
    };
    // El hilo compara los uevents con este nombre; el original se lo queda el
    // receptor del bucle principal.
    let nombre = dispositivo.clone();
    let (emisor, receptor) = channel::channel::<u32>();
    let registrado = state
        .loop_handle
        .insert_source(receptor, move |evento, _, state| {
            let channel::Event::Msg(nivel) = evento else {
                return;
            };
            // Para el brillo automático cuenta como un cambio a mano, igual que la
            // tecla: si no, lo deshacería en la siguiente lectura del sensor.
            crate::brillo_auto::tecla_teclado(state, &dispositivo, nivel);
            let porciento = (nivel * 100 / maximo.max(1)).min(100) as u8;
            avisar(state, ("teclado", Some(porciento), None));
        });
    if let Err(err) = registrado {
        tracing::warn!("sin aviso de la luz del teclado: {err}");
        return;
    }
    let lanzado = std::thread::Builder::new()
        .name("bookos-luz-teclado".into())
        .spawn(move || {
            use smithay::reexports::rustix::event::{PollFd, PollFlags, poll};
            use smithay::reexports::udev::{EventType, MonitorBuilder};
            let monitor = MonitorBuilder::new()
                .and_then(|m| m.match_subsystem("leds"))
                .and_then(|m| m.listen());
            let monitor = match monitor {
                Ok(monitor) => monitor,
                Err(err) => {
                    tracing::warn!("sin aviso de la luz del teclado: {err}");
                    return;
                }
            };
            let mut fichero = fichero;
            let leer = |fichero: &std::fs::File| {
                let mut buf = [0u8; 16];
                let n = smithay::reexports::rustix::io::pread(fichero, &mut buf, 0).ok()?;
                std::str::from_utf8(&buf[..n])
                    .ok()?
                    .trim()
                    .parse::<u32>()
                    .ok()
            };
            // sysfs exige una lectura antes del primer `poll`, o vuelve en
            // seguida; antes del primer cambio falla con ENODATA y da igual.
            let _ = leer(&fichero);
            loop {
                let mut fds = [
                    PollFd::new(&fichero, PollFlags::PRI | PollFlags::ERR),
                    PollFd::new(&monitor, PollFlags::IN),
                ];
                if poll(&mut fds, None).is_err() {
                    return;
                }
                let (luz, uevent) = (!fds[0].revents().is_empty(), !fds[1].revents().is_empty());
                if uevent {
                    // `count` y no `any`: hay que vaciar la cola entera, o lo
                    // que quede sin leer vuelve a despertar el `poll`.
                    let renacido = monitor
                        .iter()
                        .filter(|evento| {
                            evento.event_type() == EventType::Add
                                && evento.sysname() == nombre.as_str()
                        })
                        .count();
                    if renacido > 0 {
                        match std::fs::File::open(&ruta) {
                            Ok(nuevo) => {
                                fichero = nuevo;
                                let _ = leer(&fichero);
                                tracing::info!("luz del teclado reabierta tras recargar el driver");
                            }
                            Err(err) => {
                                tracing::warn!("no se pudo reabrir la luz del teclado: {err}")
                            }
                        }
                    }
                }
                if luz
                    && let Some(nivel) = leer(&fichero)
                    && emisor.send(nivel).is_err()
                {
                    // El compositor se ha ido: no hay a quién avisar.
                    return;
                }
            }
        });
    if let Err(err) = lanzado {
        tracing::warn!("sin aviso de la luz del teclado: {err}");
    }
}

/// Programa **un** despertar para cuando el aviso empiece a irse.
///
/// Sin esto, el compositor se duerme con el OSD en pantalla y no vuelve a
/// dibujar: el aviso se quedaría puesto hasta que otra cosa provocara un
/// frame. Con esto es un solo despertar, no un temporizador que repita.
// 0.1.0
fn despertar_para_la_salida(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let Some(queda) = state.shell.as_ref().and_then(|s| s.osd_queda()) else {
        return;
    };
    let hasta_la_salida = queda.saturating_sub(bookos_shell::osd::SALIDA);
    let result =
        state
            .loop_handle
            .insert_source(Timer::from_duration(hasta_la_salida), |_, _, state| {
                state.needs_redraw = true;
                TimeoutAction::Drop
            });
    if let Err(err) = result {
        tracing::error!("no se pudo programar la salida del aviso: {err}");
    }
}

/// Sube o baja el volumen y devuelve cómo ha quedado.
///
/// Se lee **antes** de escribir en vez de llevar la cuenta aquí: el volumen lo
/// puede haber cambiado otro programa, y sumarle el paso a un número viejo da
/// saltos raros.
// 0.1.0
fn volumen(paso: i32) {
    bookos_system::request(bookos_system::Operation::VolumeStep { step: paso });
}

// 0.1.0
fn silenciar(destino: &str) {
    bookos_system::request(bookos_system::Operation::Mute {
        target: destino.into(),
        muted: None,
    });
}

// 0.1.0
pub fn recibir_sistema(state: &mut BookosComp) {
    use bookos_system::Operation;
    let mut mostro_osd = false;
    while let Some((operation, result)) = bookos_system::take_feedback() {
        if result.is_ok() && matches!(operation, Operation::VolumeStep { .. }) {
            match state.chasquido {
                Chasquido::Libre => chasquido(state),
                Chasquido::Retenido => {}
                Chasquido::Aplazado(_) => soltar_chasquido(state),
            }
        }
        let osd = match result {
            Err(e) => Some(("sin-red", None, Some(e))),
            Ok(()) => match operation {
                Operation::VolumeStep { .. }
                | Operation::Volume { .. }
                | Operation::Mute { .. } => {
                    let input = matches!(&operation,Operation::Mute{target,..}|Operation::Volume{target,..} if target=="input"||target=="@DEFAULT_AUDIO_SOURCE@");
                    bookos_system::volume(input).map(|(level, muted)| {
                        (
                            if input {
                                if muted { "micro-silencio" } else { "micro" }
                            } else {
                                icono_volumen(level, muted)
                            },
                            Some(if muted { 0 } else { level }),
                            None,
                        )
                    })
                }
                _ => None,
            },
        };
        if let (Some(shell), Some((icon, level, text))) = (state.shell.as_mut(), osd) {
            shell.mostrar_osd(icon, level, text);
            mostro_osd = true;
        }
    }
    // El volumen que cambia sin pasar por aquí: el gesto de unos auriculares
    // Bluetooth (AVRCP lo lleva hasta PipeWire), u otra aplicación. Antes no
    // salía nada, como si no hubiera cambiado.
    let ahora = bookos_system::volume(false);
    let anterior = std::mem::replace(&mut state.volumen_visto, ahora);
    // Con una tarjeta abierta no: el deslizador del centro de control mueve el
    // volumen y el aviso saldría encima de lo que se está tocando. Tampoco la
    // primera lectura, que no es un cambio.
    let tarjeta = state
        .shell
        .as_ref()
        .is_some_and(|s| s.emergente_nombre().is_some());
    if !mostro_osd
        && !tarjeta
        && let (Some(antes), Some((nivel, silenciado))) = (anterior, ahora)
        && antes != (nivel, silenciado)
        && let Some(shell) = state.shell.as_mut()
    {
        shell.mostrar_osd(
            icono_volumen(nivel, silenciado),
            Some(if silenciado { 0 } else { nivel }),
            None,
        );
        mostro_osd = true;
    }
    if mostro_osd {
        despertar_para_la_salida(state);
    }
}

/// El sonido de KDE al cambiar el volumen: se oye a qué nivel ha quedado.
///
/// Suena aquí y no en `bookos-system` porque es el compositor quien lee
/// `panel.conf`. Suena tras aplicar el paso, no al pulsar: antes sonaría al
/// volumen viejo. Los pasos seguidos llegan ya sumados en uno.
// 0.1.0
fn chasquido(state: &mut BookosComp) {
    if !state.sonido_volumen {
        return;
    }
    sonar(state, "audio-volume-change");
}

/// Cuándo le toca sonar al volumen cuando se mueve con su tecla.
///
/// Suena al **soltar**, una vez, y no en cada paso. Con la tecla mantenida
/// los pasos llegan diez por segundo y sonaban todos: un tableteo que tapa
/// justo lo que el sonido tiene que decir, a qué nivel ha quedado. Sonar en
/// la primera pulsación tampoco vale, porque al pulsar no se sabe todavía si
/// la tecla se va a mantener.
#[derive(Debug)]
pub enum Chasquido {
    /// Nadie tiene la tecla: un cambio de volumen suena en el acto.
    Libre,
    /// Tecla de volumen pulsada: los pasos se aplican callados.
    Retenido,
    /// Tecla soltada, a la espera del último paso.
    Aplazado(smithay::reexports::calloop::RegistrationToken),
}

/// Se pulsó una tecla de volumen.
// 0.1.0
pub fn retener_chasquido(state: &mut BookosComp) {
    if let Chasquido::Aplazado(token) = std::mem::replace(&mut state.chasquido, Chasquido::Retenido)
    {
        state.loop_handle.remove(token);
    }
}

/// Se soltó la tecla, o llegó un paso con ella ya suelta: suena en cuanto
/// dejen de llegar.
///
/// No suena en el mismo instante de soltar porque los pasos van por una cola
/// a `bookos-system` y el último puede confirmarse después: se oiría el nivel
/// anterior, o dos veces.
// 0.1.0
pub fn soltar_chasquido(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    const MARGEN: std::time::Duration = std::time::Duration::from_millis(60);
    match std::mem::replace(&mut state.chasquido, Chasquido::Libre) {
        Chasquido::Libre => return,
        Chasquido::Retenido => {}
        Chasquido::Aplazado(token) => state.loop_handle.remove(token),
    }
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(MARGEN), |_, _, state| {
            state.chasquido = Chasquido::Libre;
            chasquido(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(token) => state.chasquido = Chasquido::Aplazado(token),
        Err(err) => tracing::warn!("sin sonido al soltar la tecla de volumen: {err}"),
    }
}

/// Un sonido de los que acompañan un gesto (bloquear, desbloquear, capturar,
/// iniciar sesión), si `sonidos_interfaz` está puesto.
// 0.1.0
pub fn sonar_interfaz(state: &mut BookosComp, nombre: &str) {
    if state.sonidos_interfaz {
        sonar(state, nombre);
    }
}

/// Dónde está el sonido de un evento: en el tema de BookOS y, si no lo tiene,
/// en el de freedesktop, que es de quien hereda (ver su `index.theme`).
///
/// Va a mano y no por libcanberra porque aquí no se enlaza contra ella y
/// `canberra-gtk-play` tarda en arrancar más que `pw-play`. Antes la ruta de
/// freedesktop estaba escrita tal cual, y con el tema de BookOS instalado el
/// volumen y el cargador seguían sonando con los de serie.
// 0.1.0
fn ruta_sonido(nombre: &str) -> Option<std::path::PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        bases.push(std::path::PathBuf::from(home).join(".local/share/sounds"));
    }
    bases.push("/usr/local/share/sounds".into());
    bases.push("/usr/share/sounds".into());
    ["bookos", "freedesktop"].into_iter().find_map(|tema| {
        bases.iter().find_map(|base| {
            ["oga", "ogg", "wav"]
                .into_iter()
                .map(|ext| {
                    base.join(tema)
                        .join("stereo")
                        .join(format!("{nombre}.{ext}"))
                })
                .find(|ruta| ruta.is_file())
        })
    })
}

/// Toca un sonido del tema sin esperar a que acabe.
// 0.1.0
pub fn sonar(state: &mut BookosComp, nombre: &str) {
    state
        .hijos
        .retain_mut(|hijo| !matches!(hijo.try_wait(), Ok(Some(_))));
    let Some(ruta) = ruta_sonido(nombre) else {
        tracing::warn!(nombre, "sin sonido: no está en ningún tema");
        return;
    };
    // modificado en la 0.2.0 -- conserva la medida del arranque antes de plantear un flujo propio.
    let desde = std::time::Instant::now();
    let resultado = std::process::Command::new("pw-play")
        .arg(ruta)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    tracing::debug!(
        nombre,
        arranque_ms = desde.elapsed().as_secs_f64() * 1000.0,
        "arranque del sonido de interfaz"
    );
    match resultado {
        Ok(hijo) => state.hijos.push(hijo),
        Err(err) => tracing::warn!(nombre, "sin sonido: {err}"),
    }
}

/// Suena y avisa al conectar o desconectar el cargador.
///
/// Se llama con cada aviso de udev de `power_supply`, que llegan en ráfagas
/// (el adaptador y la batería por separado, y luego cada cambio de
/// porcentaje): por eso se compara con lo último visto y no se avisa por
/// evento. Se mira el estado de la batería y no solo el del adaptador porque
/// es lo que ya usa el widget del panel, y así los dos nunca se contradicen.
// 0.1.0
// modificado en la 0.2.0 -- en el idioma de la sesión, como el resto del shell
pub fn cargador(state: &mut BookosComp) {
    use bookos_shell::t;
    let Some((enchufado, porciento)) = bookos_shell::cargador() else {
        return;
    };
    if state.enchufado.replace(enchufado) == Some(enchufado) {
        return;
    }
    let (resumen, cuerpo, sonido) = if enchufado {
        (
            t("Cargador conectado", "Charger connected"),
            format!("{porciento} % · {}", t("cargando", "charging")),
            "power-plug",
        )
    } else {
        (
            t("Cargador desconectado", "Charger disconnected"),
            format!("{porciento} % {}", t("de batería", "battery")),
            "power-unplug",
        )
    };
    tracing::info!(enchufado, porciento, "cambio de alimentación");
    sonar(state, sonido);
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    let mut notificacion = bookos_shell::notificaciones::Notificacion::nueva(
        0,
        "BookOS".into(),
        resumen.into(),
        cuerpo,
        "",
        false,
    );
    // El del panel y no uno del tema, para que el aviso y la barra se vean
    // iguales. Si la batería desapareció entre las dos lecturas se queda el
    // genérico de notificaciones que ya puso `nueva`.
    if let Some(icono) = bookos_shell::icono_cargador() {
        notificacion.icono = Some(icono);
    }
    shell.notificar(notificacion, 3000);
    state.needs_redraw = true;
}

/// Recoge cada segundo lo que el hilo del centro de control ha leído de MPRIS
/// y repinta si ha cambiado: canción, carátula, avance, pausa.
///
/// Se mira en cada fotograma si hace falta, y no al abrir la tarjeta, porque
/// hay muchos caminos para abrirla (clic, atajo, cambio desde otra tarjeta) y
/// todos acaban pintando. El temporizador se suelta solo al cerrarla: con el
/// centro cerrado no hay nada despertando cada segundo.
// 0.1.0
pub fn vigilar_centro(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    const CADA: std::time::Duration = std::time::Duration::from_secs(1);
    let abierto = |state: &BookosComp| {
        state.shell.as_ref().and_then(|s| s.emergente_nombre()) == Some("centro")
    };
    if state.tick_centro.is_some() || !abierto(state) {
        return;
    }
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(CADA), move |_, _, state| {
            if !abierto(state) {
                state.tick_centro = None;
                return TimeoutAction::Drop;
            }
            if state.shell.as_mut().is_some_and(|s| s.refresh_emergente()) {
                state.needs_redraw = true;
            }
            TimeoutAction::ToDuration(CADA)
        });
    match token {
        Ok(token) => state.tick_centro = Some(token),
        Err(err) => tracing::warn!("sin segundero para el centro de control: {err}"),
    }
}

// 0.1.0
fn icono_volumen(nivel: u8, silenciado: bool) -> &'static str {
    match (silenciado, nivel) {
        (true, _) | (_, 0) => "volumen-silencio",
        (_, n) if n < 40 => "volumen-bajo",
        (_, n) if n < 75 => "volumen-medio",
        _ => "volumen-alto",
    }
}

/// Sube o baja el brillo de la pantalla por logind, que es quien tiene el
/// permiso: `/sys/class/backlight/*/brightness` es de root.
// 0.1.0
// modificado en la 0.2.0 -- el cambio va por poner_brillo, compartido con el buscador
fn brillo(state: &mut BookosComp, paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let Some(actual) = crate::brillo_auto::partida_tecla(state) else {
        return (
            "brillo",
            None,
            Some("Brillo de pantalla no disponible".into()),
        );
    };
    poner_brillo(state, (actual as i32 + paso).clamp(5, 100) as u8)
}

/// Pone el brillo en `nivel` %, como si se hubiera llegado con las teclas: el
/// brillo automático lo toma por un ajuste a mano. Lo usan las teclas y el
/// buscador («brillo 30»).
// añadido en la 0.2.0
pub fn poner_brillo(
    state: &mut BookosComp,
    nivel: u8,
) -> (&'static str, Option<u8>, Option<String>) {
    let nuevo = nivel.clamp(5, 100);
    let Some((dispositivo, maximo)) = bookos_shell::backlight() else {
        return (
            "brillo",
            None,
            Some("Brillo de pantalla no disponible".into()),
        );
    };
    let crudo = (maximo as f64 * nuevo as f64 / 100.0).round() as u32;
    bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
    crate::brillo_auto::tecla(state, nuevo);
    ("brillo", Some(nuevo), None)
}

/// El teclado tiene su propia retroiluminación, con niveles enteros (0..max) y
/// no un tanto por ciento: en este portátil son cuatro pasos.
// 0.1.0
fn brillo_teclado(state: &mut BookosComp, paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return ("teclado", None, Some("Sin luz de teclado".into()));
    };
    let actual = crate::brillo_auto::partida_tecla_teclado(state, &dispositivo, actual);
    let nuevo = (i64::from(actual) + i64::from(paso)).clamp(0, i64::from(maximo)) as u32;
    bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nuevo);
    crate::brillo_auto::tecla_teclado(state, &dispositivo, nuevo);
    // El nivel se enseña en tanto por ciento para que la barra diga algo: con
    // cuatro pasos, "1 de 3" no se lee de un vistazo.
    let porciento = (u64::from(nuevo) * 100 / u64::from(maximo.max(1))) as u8;
    ("teclado", Some(porciento), None)
}

// 0.1.0
fn alternar_brillo_teclado(state: &mut BookosComp) -> (&'static str, Option<u8>, Option<String>) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return ("teclado", None, Some("Sin luz de teclado".into()));
    };
    let actual = crate::brillo_auto::partida_tecla_teclado(state, &dispositivo, actual);
    let nuevo = if actual == 0 { 1.min(maximo) } else { 0 };
    bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nuevo);
    crate::brillo_auto::tecla_teclado(state, &dispositivo, nuevo);
    let porciento = (u64::from(nuevo) * 100 / u64::from(maximo.max(1))) as u8;
    ("teclado", Some(porciento), None)
}

// 0.1.0
fn camara(estado: Option<bool>) -> (&'static str, Option<u8>, Option<String>) {
    let (icono, texto) = match estado {
        Some(true) => ("camara", "Cámara activada"),
        Some(false) => ("camara-desactivado", "Cámara desactivada"),
        None => ("camara", "Cámara"),
    };
    // Icono propio: `camera-web` es del tema del sistema y en esta máquina no
    // resolvía a nada, así que el aviso salía sin dibujo.
    (icono, None, Some(texto.into()))
}

/// Enciende y apaga el touchpad. El cambio se guarda en el estado para que los
/// dispositivos que aparezcan después —tras un cambio de TTY— nazcan igual.
// 0.1.0
fn touchpad(state: &mut BookosComp) -> (&'static str, Option<u8>, Option<String>) {
    state.touchpad_activo = !state.touchpad_activo;
    // A los que ya están abiertos hay que decírselo ahora: `DeviceAdded` solo
    // llega con los que aparecen, y el touchpad del portátil apareció al
    // arrancar. El cierre lo pone el backend de sesión real; anidado no hay
    // libinput y no hay nada que aplicar.
    if let Some(aplicar) = state.aplicar_touchpad.as_ref() {
        aplicar(state.touchpad_activo);
    }
    let (icono, texto) = if state.touchpad_activo {
        ("touchpad", "Touchpad activado")
    } else {
        ("touchpad-desactivado", "Touchpad desactivado")
    };
    (icono, None, Some(texto.into()))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Los bytes tal cual los dio `hexdump -C /dev/input/event3` en el Book5 Pro
    /// al pulsar Fn+F11 dos veces: bloquear, un `SYN_REPORT` y desbloquear.
    #[test]
    fn la_tapa_de_la_camara_se_lee_como_la_manda_el_driver() {
        let volcado: [u8; 96] = [
            0xcd, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, 0x74, 0xed, 0x0b, 0, 0, 0, 0, 0, //
            0x05, 0, 0x09, 0, 0x01, 0, 0, 0, 0xcd, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, //
            0x74, 0xed, 0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
            0xce, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, 0xbd, 0x42, 0x0c, 0, 0, 0, 0, 0, //
            0x05, 0, 0x09, 0, 0, 0, 0, 0, 0xce, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, //
            0xbd, 0x42, 0x0c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let lecturas: Vec<_> = volcado
            .as_chunks::<EVENTO>()
            .0
            .iter()
            .map(camara_activa)
            .collect();
        assert_eq!(lecturas, [Some(false), None, Some(true), None]);
    }
}
