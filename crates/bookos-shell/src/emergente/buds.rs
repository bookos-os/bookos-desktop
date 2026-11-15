//! Los Galaxy Buds vistos desde la tarjeta de Bluetooth: batería de cada pieza
//! y control de ruido, que habla `bookos-budsd` (`org.bookos.Buds`).
//!
//! Se llama con `busctl` y no con una dependencia de D-Bus porque el shell no
//! tiene ninguna y es lo que ya hace `medios.rs` con MPRIS. Cada llamada es un
//! proceso, así que nunca se hace en el hilo del compositor: las lecturas van
//! en un hilo que deja el resultado en un [`Buzon`], y las órdenes se lanzan
//! sin esperar.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::Value;

/// Lo que se enseña de unos Buds. Una batería a 0 es «no la sé»: budsd manda 0
/// cuando el aparato no ha contestado todavía.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Estado {
    pub anc: Modo,
    pub izquierdo: u8,
    pub derecho: u8,
    pub estuche: u8,
}

/// El control de ruido. El adaptativo de los Buds3 Pro no se ofrece: en el
/// escritorio se elige entre quitar el ruido, dejar pasar el ambiente o nada,
/// y el cuarto estado de budsd se conserva al leerlo como «Cancelación».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modo {
    Cancelacion,
    Ambiente,
    Apagado,
}

impl Modo {
    pub const TODOS: [Modo; 3] = [Modo::Cancelacion, Modo::Ambiente, Modo::Apagado];

    /// El número de `SetAnc`: 0 apagado, 1 cancelación, 2 ambiente.
    fn numero(self) -> u8 {
        match self {
            Modo::Apagado => 0,
            Modo::Cancelacion => 1,
            Modo::Ambiente => 2,
        }
    }

    fn de(numero: u64) -> Self {
        match numero {
            0 => Modo::Apagado,
            2 => Modo::Ambiente,
            _ => Modo::Cancelacion,
        }
    }

    pub fn etiqueta(self) -> &'static str {
        match self {
            Modo::Cancelacion => "Cancelación",
            Modo::Ambiente => "Ambiente",
            Modo::Apagado => "No",
        }
    }
}

/// Donde el hilo deja lo último que leyó. `None`: budsd no está, o no hay
/// unos Buds con el canal abierto.
pub type Buzon = Arc<Mutex<Option<Estado>>>;

// añadido en la 0.2.0
fn busctl(metodo: &[&str]) -> Command {
    let mut orden = Command::new("busctl");
    orden
        .args([
            "--user",
            "call",
            "org.bookos.Buds",
            "/org/bookos/Buds",
            "org.bookos.Buds",
        ])
        .args(metodo)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    orden
}

/// Lee el estado de budsd. `None` si no contesta o dice que no hay canal.
// añadido en la 0.2.0
fn consultar() -> Option<Estado> {
    let salida = busctl(&["GetStatus"]).output().ok()?;
    if !salida.status.success() {
        return None;
    }
    let json = crate::medios::cadena(&String::from_utf8_lossy(&salida.stdout))?;
    interpretar(&json)
}

/// Del JSON de budsd (`BudsStatus`) a lo que se enseña.
// añadido en la 0.2.0
fn interpretar(json: &str) -> Option<Estado> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v["connected"] != true {
        return None;
    }
    let pila = |clave: &str| {
        v[clave]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(0)
    };
    Some(Estado {
        anc: Modo::de(v["anc_mode"].as_u64()?),
        izquierdo: pila("battery_l"),
        derecho: pila("battery_r"),
        estuche: pila("battery_case"),
    })
}

/// Cambia el control de ruido sin esperar la respuesta.
// añadido en la 0.2.0
pub fn poner(modo: Modo) {
    let numero = modo.numero().to_string();
    let Ok(mut hijo) = busctl(&["SetAnc", "y", &numero])
        .stdout(Stdio::null())
        .spawn()
    else {
        return;
    };
    // Recoger al hijo: sin `wait` quedaría zombi hasta que muera el shell.
    std::thread::spawn(move || {
        let _ = hijo.wait();
    });
}

/// Relee budsd cada 3 s mientras el buzón viva. Solo guarda un `Weak`: al
/// cerrarse la tarjeta el `Arc` muere con ella y el hilo termina en la vuelta
/// siguiente. 3 s y no 1 porque cada `GetStatus` le pide el estado al aparato
/// por radio, y la batería no cambia a ese ritmo.
// añadido en la 0.2.0
pub fn vigilar(buzon: &Buzon) {
    let destino: Weak<_> = Arc::downgrade(buzon);
    let resultado = std::thread::Builder::new()
        .name("bookos-centro-buds".into())
        .spawn(move || {
            loop {
                let leido = consultar();
                let Some(destino) = destino.upgrade() else {
                    return;
                };
                *destino.lock().expect("nadie suelta el cerrojo a medias") = leido;
                drop(destino);
                std::thread::sleep(Duration::from_secs(3));
            }
        });
    if let Err(error) = resultado {
        tracing::warn!("sin hilo para leer los Buds: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El JSON real de budsd se lee, y sin canal no hay estado: enseñar los
    /// controles de unos Buds que no contestan sería un interruptor mentiroso.
    #[test]
    fn lee_lo_que_manda_budsd() {
        let con =
            r#"{"connected":true,"battery_l":82,"battery_r":79,"battery_case":45,"anc_mode":2}"#;
        assert_eq!(
            interpretar(con),
            Some(Estado {
                anc: Modo::Ambiente,
                izquierdo: 82,
                derecho: 79,
                estuche: 45
            })
        );
        assert_eq!(interpretar(r#"{"connected":false,"anc_mode":1}"#), None);
        assert_eq!(interpretar("no es json"), None);
    }

    /// El modo adaptativo (3) se muestra como cancelación en vez de dejar el
    /// selector sin nada marcado.
    #[test]
    fn el_adaptativo_se_lee_como_cancelacion() {
        assert_eq!(Modo::de(3), Modo::Cancelacion);
        assert_eq!(Modo::Cancelacion.numero(), 1);
        assert_eq!(Modo::Apagado.numero(), 0);
    }
}
