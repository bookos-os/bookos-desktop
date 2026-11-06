//! Pellizco de cinco dedos, que libinput no emite.
//!
//! libinput 1.31.3 solo procesa gestos hasta cuatro dedos
//! (`evdev-mt-touchpad-gestures.c:2076`), aunque este touchpad entrega cinco
//! contactos: medido con `BTN_TOOL_QUINTTAP` y cinco `ABS_MT_TRACKING_ID`.
//!
//! El estado se consulta con `EVIOCGMTSLOTS` sobre un `dup` del descriptor que
//! abre libinput. Un `read` robaría eventos, porque el `dup` comparte la cola;
//! un segundo `open` por la sesión se descartó para no pelear por el
//! dispositivo con libseat.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Instant;

use smithay::backend::libinput::LibinputSessionInterface;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use smithay::reexports::input::LibinputInterface;

use crate::gestos::Gestos;
use crate::state::BookosComp;

const ABS_MT_SLOT: u32 = 0x2f;
const ABS_MT_POSITION_X: u32 = 0x35;
const ABS_MT_POSITION_Y: u32 = 0x36;
const ABS_MT_TRACKING_ID: u32 = 0x39;
const INPUT_PROP_POINTER: u8 = 1 << 0;
const INPUT_PROP_DIRECT: u8 = 1 << 1;
const DEDOS: usize = 5;
// Más que los 5 que declara este touchpad: el kernel copia solo los que tenga y
// deja el resto del búfer intacto (`evdev_handle_mt_request`), sin decir cuántos
// escribió. Ver `consulta`.
const HUECOS: usize = 10;

const fn lectura_evdev(numero: u32, bytes: usize) -> libc::c_ulong {
    ((2u32 << 30) | ((bytes as u32) << 16) | (0x45 << 8) | numero) as libc::c_ulong
}

/// El búfer de `EVIOCGMTSLOTS`, con los huecos a -1 («libre»): a cero, los que
/// el kernel no toca contarían como dedos apoyados, y un touchpad de cinco
/// huecos parecería tener siempre cinco dedos encima.
fn consulta(codigo: u32) -> [i32; HUECOS + 1] {
    let mut datos = [-1; HUECOS + 1];
    datos[0] = codigo as i32;
    datos
}

fn dedos_apoyados(ids: &[i32]) -> usize {
    ids.iter().filter(|id| **id >= 0).count()
}

/// La distancia media al centro: a diferencia de la de dos dedos que usa
/// libinput, no depende de qué dedos quedan en los extremos.
fn radio(puntos: &[(f64, f64)]) -> f64 {
    let n = puntos.len() as f64;
    let (cx, cy) = puntos
        .iter()
        .fold((0.0, 0.0), |(x, y), (px, py)| (x + px, y + py));
    let (cx, cy) = (cx / n, cy / n);
    puntos
        .iter()
        .map(|(x, y)| (x - cx).hypot(y - cy))
        .sum::<f64>()
        / n
}

#[derive(Default)]
pub struct Cinco {
    // Instancia aparte de `BookosComp::gestos`: si libinput ya empezó un gesto
    // de cuatro dedos, tiene que poder cerrarlo con el cliente sin pisar este.
    gestos: Gestos,
    radio_inicial: Option<f64>,
}

#[derive(Clone, Copy)]
struct Touchpad {
    fd: RawFd,
    // Unidades por milímetro de cada eje: no tienen por qué coincidir, y sin
    // corregirlas un pellizco en diagonal mediría distinto según el touchpad.
    resolucion: (f64, f64),
}

impl Touchpad {
    /// Solo cuentan los dispositivos de puntero multitáctil con cinco huecos;
    /// una pantalla táctil lleva `INPUT_PROP_DIRECT` y va por su propio camino.
    fn de(fd: RawFd) -> Option<Self> {
        let mut propiedades = [0u8; 4];
        // SAFETY: `fd` está abierto y el búfer mide lo que declara la petición.
        let leido = unsafe {
            libc::ioctl(
                fd,
                lectura_evdev(0x09, propiedades.len()),
                propiedades.as_mut_ptr(),
            )
        };
        if leido < 0
            || propiedades[0] & INPUT_PROP_POINTER == 0
            || propiedades[0] & INPUT_PROP_DIRECT != 0
        {
            return None;
        }
        let huecos = eje(fd, ABS_MT_SLOT)?;
        if huecos[2] + 1 < DEDOS as i32 {
            return None;
        }
        let resolucion = |codigo| eje(fd, codigo).map(|e| f64::from(e[5].max(1)));
        Some(Self {
            fd,
            resolucion: (
                resolucion(ABS_MT_POSITION_X)?,
                resolucion(ABS_MT_POSITION_Y)?,
            ),
        })
    }

    fn huecos(self, codigo: u32) -> std::io::Result<[i32; HUECOS]> {
        let mut datos = consulta(codigo);
        // SAFETY: `fd` lo posee la fuente que llama y el búfer mide lo declarado.
        let leido = unsafe {
            libc::ioctl(
                self.fd,
                lectura_evdev(0x0a, size_of_val(&datos)),
                datos.as_mut_ptr(),
            )
        };
        if leido < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut valores = [0; HUECOS];
        valores.copy_from_slice(&datos[1..]);
        Ok(valores)
    }

    /// `None` mientras no haya cinco dedos apoyados. Esta consulta corre a cada
    /// evento del touchpad, así que las posiciones se piden solo si hacen falta.
    fn radio_de_cinco(self) -> std::io::Result<Option<f64>> {
        let ids = self.huecos(ABS_MT_TRACKING_ID)?;
        if dedos_apoyados(&ids) < DEDOS {
            return Ok(None);
        }
        let xs = self.huecos(ABS_MT_POSITION_X)?;
        let ys = self.huecos(ABS_MT_POSITION_Y)?;
        let puntos: Vec<_> = ids
            .iter()
            .zip(xs.iter().zip(&ys))
            .filter(|(id, _)| **id >= 0)
            .map(|(_, (x, y))| {
                (
                    f64::from(*x) / self.resolucion.0,
                    f64::from(*y) / self.resolucion.1,
                )
            })
            .collect();
        Ok(Some(radio(&puntos)))
    }
}

fn eje(fd: RawFd, codigo: u32) -> Option<[i32; 6]> {
    // `input_absinfo`: valor, mínimo, máximo, fuzz, flat, resolución.
    let mut info = [0i32; 6];
    // SAFETY: `fd` está abierto y el búfer mide lo que declara la petición.
    let leido = unsafe {
        libc::ioctl(
            fd,
            lectura_evdev(0x40 + codigo, size_of_val(&info)),
            info.as_mut_ptr(),
        )
    };
    (leido >= 0).then_some(info)
}

fn actualizar(state: &mut BookosComp, radio: Option<f64>, cancelar: bool, ahora: u64) {
    let bloqueado = state.shell.as_ref().is_none_or(|s| s.esta_bloqueado());
    let permitido = !cancelar && !state.modo.kiosko() && !bloqueado;
    match (state.cinco_dedos.radio_inicial, radio.filter(|_| permitido)) {
        (None, Some(radio)) if radio > 0.0 => {
            state.cinco_dedos.radio_inicial = Some(radio);
            state
                .cinco_dedos
                .gestos
                .pellizco_inicio(DEDOS as u32, ahora);
            if let Some(shell) = state.shell.as_mut() {
                shell.agarrar_launchpad();
            }
        }
        (Some(inicial), Some(radio)) => {
            if let Some(progreso) = state
                .cinco_dedos
                .gestos
                .pellizco_avance(radio / inicial, ahora)
                && let Some(shell) = state.shell.as_mut()
            {
                shell.seguir_launchpad(progreso);
                state.needs_redraw = true;
            }
        }
        (Some(_), None) => {
            state.cinco_dedos.radio_inicial = None;
            // Levantar la mano es soltar; perder el permiso o el dispositivo no
            // es una decisión del usuario y no debe abrir nada.
            let cancelada = !state.modo.kiosko() && !bloqueado && !cancelar;
            if let Some(suelta) = state.cinco_dedos.gestos.pellizco_fin(!cancelada, ahora)
                && let Some(shell) = state.shell.as_mut()
            {
                shell.soltar_launchpad(suelta);
                state.needs_redraw = true;
            }
        }
        (None, _) => {}
    }
}

/// Envuelve la interfaz de la sesión porque es el único sitio donde libinput
/// enseña sus descriptores, y también donde sabemos que se reabren al volver
/// de otra VT.
pub struct Interfaz {
    sesion: LibinputSessionInterface<LibSeatSession>,
    bucle: LoopHandle<'static, BookosComp>,
    fuentes: HashMap<RawFd, RegistrationToken>,
    inicio: Instant,
}

impl Interfaz {
    pub fn new(sesion: LibSeatSession, bucle: LoopHandle<'static, BookosComp>) -> Self {
        Self {
            sesion: sesion.into(),
            bucle,
            fuentes: HashMap::new(),
            inicio: Instant::now(),
        }
    }

    fn vigilar(&mut self, original: RawFd, copia: OwnedFd) -> Result<(), String> {
        let Some(touchpad) = Touchpad::de(copia.as_raw_fd()) else {
            return Ok(());
        };
        let inicio = self.inicio;
        // El mismo aviso de lectura que despierta a libinput: sin dedos encima
        // no hay ni un despertar, que es por lo que se descartó un temporizador.
        let fuente = Generic::new(copia, Interest::READ, Mode::Level);
        let token = self
            .bucle
            .insert_source(fuente, move |_, _, state| {
                let ahora = inicio.elapsed().as_micros() as u64;
                match touchpad.radio_de_cinco() {
                    Ok(radio) => {
                        actualizar(state, radio, false, ahora);
                        Ok(PostAction::Continue)
                    }
                    // Un fd revocado queda legible para siempre: seguir en
                    // modo Level sería un bucle activo hasta que libinput lo cierre.
                    Err(err) => {
                        tracing::debug!("touchpad sin leer: {err}");
                        actualizar(state, None, true, ahora);
                        Ok(PostAction::Remove)
                    }
                }
            })
            .map_err(|err| err.to_string())?;
        self.fuentes.insert(original, token);
        Ok(())
    }
}

impl LibinputInterface for Interfaz {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let fd = self.sesion.open_restricted(path, flags)?;
        // Sin el gesto de cinco dedos el touchpad sigue siendo útil: un fallo
        // aquí no debe impedir que libinput abra el dispositivo.
        let vigilado = fd
            .try_clone()
            .map_err(|err| err.to_string())
            .and_then(|copia| self.vigilar(fd.as_raw_fd(), copia));
        if let Err(err) = vigilado {
            tracing::warn!("sin pellizco de cinco dedos en {}: {err}", path.display());
        }
        Ok(fd)
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        if let Some(token) = self.fuentes.remove(&fd.as_raw_fd()) {
            self.bucle.remove(token);
        }
        self.sesion.close_restricted(fd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_radio_escala_con_la_mano_y_no_con_su_posicion() {
        let mano = [
            (0.0, 0.0),
            (10.0, 0.0),
            (20.0, 5.0),
            (30.0, 0.0),
            (15.0, 20.0),
        ];
        let lejos = mano.map(|(x, y)| (x + 50.0, y + 30.0));
        let doble = mano.map(|(x, y)| (x * 2.0, y * 2.0));
        assert!((radio(&mano) - radio(&lejos)).abs() < 1e-9);
        assert!((radio(&doble) / radio(&mano) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn un_dedo_en_touchpad_de_cinco_huecos_no_cuenta_como_cinco() {
        let mut datos = consulta(ABS_MT_TRACKING_ID);
        assert_eq!(datos[0], ABS_MT_TRACKING_ID as i32);
        // Lo que escribe el kernel: solo los cinco huecos que tiene el dispositivo.
        datos[1..=5].copy_from_slice(&[42, -1, -1, -1, -1]);
        assert_eq!(dedos_apoyados(&datos[1..]), 1);
    }

    #[test]
    fn las_peticiones_ioctl_coinciden_con_las_del_kernel() {
        // EVIOCGABS(ABS_MT_SLOT), ya usado contra el touchpad real.
        assert_eq!(lectura_evdev(0x40 + ABS_MT_SLOT, 24), 0x8018_456f);
        assert_eq!(lectura_evdev(0x0a, 44), 0x802c_450a);
    }
}
