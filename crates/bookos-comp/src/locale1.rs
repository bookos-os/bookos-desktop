//! La distribución de teclado de systemd-localed, para el modo kiosko.
//!
//! En el escritorio el teclado lo dice `panel.conf`. En el kiosko no hay
//! usuario ni panel: el asistente de primer arranque cambia la distribución con
//! `localectl set-x11-keymap` y el campo donde se prueba tiene que escribir ya
//! con la nueva, sin reiniciar nada. Es lo que hacía `kwin --locale1`, que era
//! quien alojaba antes el asistente.
//!
//! No vale el `KEYMAP` de `/etc/vconsole.conf` que usa
//! [`crate::state::distribucion_teclado`]: es un mapa de consola
//! (`es-winkeys`, `la-latin1`…), no siempre existe en xkb, y no lleva variante.

use smithay::input::keyboard::XkbConfig;
use smithay::reexports::calloop::LoopHandle;
use zbus::blocking::{Connection, Proxy};

use crate::state::BookosComp;

const LOCALED: &str = "org.freedesktop.locale1";
const RUTA: &str = "/org/freedesktop/locale1";

/// Distribución y variante, tal como las guarda localed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Distribucion {
    pub layout: String,
    pub variant: String,
}

impl Distribucion {
    pub fn xkb(&self) -> XkbConfig<'_> {
        XkbConfig {
            layout: &self.layout,
            variant: &self.variant,
            ..Default::default()
        }
    }
}

/// Sin caché: el proxy de zbus guarda las propiedades y solo las refresca con
/// las señales que él mismo ve, y aquí se lee justo después de una.
fn proxy(conexion: &Connection) -> zbus::Result<Proxy<'static>> {
    zbus::blocking::proxy::Builder::<Proxy>::new(conexion)
        .destination(LOCALED)?
        .path(RUTA)?
        .interface(LOCALED)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn leer(proxy: &Proxy) -> Option<Distribucion> {
    let layout: String = proxy.get_property("X11Layout").ok()?;
    let variant: String = proxy.get_property("X11Variant").unwrap_or_default();
    // Sin distribución X11 configurada localed devuelve "", y un layout vacío
    // en xkb es «us»: mejor dejar que decida la cadena de siempre.
    (!layout.trim().is_empty()).then_some(Distribucion { layout, variant })
}

/// La de ahora, si localed contesta y tiene alguna.
pub fn actual() -> Option<Distribucion> {
    let conexion = Connection::system()
        .inspect_err(|err| tracing::warn!("sin bus del sistema, no se lee locale1: {err}"))
        .ok()?;
    leer(&proxy(&conexion).ok()?)
}

/// Aplica cada cambio de distribución que haga localed mientras dure el
/// compositor.
pub fn escuchar(loop_handle: &LoopHandle<'static, BookosComp>) {
    let (aviso, fuente) = smithay::reexports::calloop::channel::channel::<Distribucion>();
    let insertado = loop_handle.insert_source(fuente, |evento, _, state| {
        if let smithay::reexports::calloop::channel::Event::Msg(nueva) = evento {
            aplicar(state, &nueva);
        }
    });
    if let Err(err) = insertado {
        tracing::error!("no se pudo escuchar locale1: {err}");
        return;
    }
    let lanzado = std::thread::Builder::new()
        .name("bookos-locale1".into())
        .spawn(move || {
            // La señal la emite la interfaz de propiedades, no la de locale1.
            // Se escucha esa y se vuelve a leer todo: `set-x11-keymap` cambia
            // distribución y variante a la vez, y un cambio solo de variante
            // (es → es cat) tampoco puede perderse.
            let escucha = Connection::system().and_then(|conexion| {
                let locale = proxy(&conexion)?;
                let propiedades =
                    Proxy::new(&conexion, LOCALED, RUTA, "org.freedesktop.DBus.Properties")?;
                Ok((locale, propiedades.receive_signal("PropertiesChanged")?))
            });
            let (locale, señales) = match escucha {
                Ok(par) => par,
                Err(err) => {
                    tracing::warn!("sin locale1, el teclado no cambiará en vivo: {err}");
                    return;
                }
            };
            let mut ultima = leer(&locale);
            for _ in señales {
                let nueva = leer(&locale);
                if nueva == ultima {
                    continue;
                }
                ultima = nueva.clone();
                if let Some(nueva) = nueva
                    && aviso.send(nueva).is_err()
                {
                    return;
                }
            }
        });
    if let Err(err) = lanzado {
        tracing::warn!("no se pudo escuchar locale1: {err}");
    }
}

fn aplicar(state: &mut BookosComp, nueva: &Distribucion) {
    let Some(teclado) = state.seat.get_keyboard() else {
        return;
    };
    match teclado.set_xkb_config(state, nueva.xkb()) {
        Ok(()) => tracing::info!(?nueva, "distribución de teclado de locale1"),
        Err(err) => tracing::warn!(?err, ?nueva, "no se pudo aplicar la distribución"),
    }
}
