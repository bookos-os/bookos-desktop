//! Montaje automático de lo que se conecta: USB, discos externos, tarjetas SD.
//!
//! En Plasma lo hacía el automontador de `kded`, que en una sesión BookOS no
//! corre: todo se quedaba sin montar hasta abrirlo a mano. Aquí se le pide a
//! udisks2, que monta en `/run/media/$USER/<etiqueta>` como el usuario de la
//! sesión y sin contraseña para lo extraíble —su política de polkit—.
//!
//! Se monta lo que ya está conectado al arrancar la sesión y, después, cada
//! volumen que aparece (`InterfacesAdded`). Desmontar a mano no lo vuelve a
//! montar: eso no añade ninguna interfaz.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

const UDISKS: &str = "org.freedesktop.UDisks2";
const BLOQUE: &str = "org.freedesktop.UDisks2.Block";
const SISTEMA_DE_ARCHIVOS: &str = "org.freedesktop.UDisks2.Filesystem";

/// Lo que hace falta saber de un volumen para decidir.
#[derive(Debug, Default, PartialEq)]
struct Volumen {
    /// `HintSystem`: udisks lo considera del sistema (discos internos).
    sistema: bool,
    /// `HintIgnore`: la regla udev de la distribución pide no enseñarlo.
    ignorar: bool,
    /// `HintAuto`: udisks recomienda montarlo solo. No se exige —en discos
    /// USB grandes viene a falso— pero cuenta como sí.
    auto: bool,
    /// Tiene una unidad física detrás. Los dispositivos de bucle (imágenes,
    /// snaps) no, y no son algo que el usuario haya enchufado.
    con_unidad: bool,
    /// Ya está montado en algún sitio.
    montado: bool,
}

/// ¿Se monta solo?
// 0.1.0
fn debe_montarse(v: &Volumen) -> bool {
    !v.montado && !v.ignorar && v.con_unidad && (!v.sistema || v.auto)
}

// 0.1.0
async fn leer(c: &Connection, ruta: &OwnedObjectPath) -> Option<Volumen> {
    let bloque = zbus::Proxy::new(c, UDISKS, ruta.as_ref(), BLOQUE)
        .await
        .ok()?;
    let fs = zbus::Proxy::new(c, UDISKS, ruta.as_ref(), SISTEMA_DE_ARCHIVOS)
        .await
        .ok()?;
    // Sin la interfaz de sistema de archivos —una partición cifrada sin abrir,
    // un disco entero con tabla de particiones— no hay nada que montar.
    let puntos: Vec<Vec<u8>> = fs.get_property("MountPoints").await.ok()?;
    let unidad: OwnedObjectPath = bloque.get_property("Drive").await.ok()?;
    Some(Volumen {
        sistema: bloque.get_property("HintSystem").await.unwrap_or(true),
        ignorar: bloque.get_property("HintIgnore").await.unwrap_or(true),
        auto: bloque.get_property("HintAuto").await.unwrap_or(false),
        con_unidad: unidad.as_str() != "/",
        montado: !puntos.is_empty(),
    })
}

// 0.1.0
async fn montar_si_toca(c: &Connection, ruta: &OwnedObjectPath) {
    let Some(volumen) = leer(c, ruta).await else {
        return;
    };
    if !debe_montarse(&volumen) {
        return;
    }
    let Ok(fs) = zbus::Proxy::new(c, UDISKS, ruta.as_ref(), SISTEMA_DE_ARCHIVOS).await else {
        return;
    };
    // Sin interacción: si polkit pidiera contraseña para esto —un disco
    // interno, una sesión que no es la activa— no se saca un diálogo por algo
    // que nadie ha pedido; se deja sin montar, como antes.
    let opciones: HashMap<&str, Value> =
        HashMap::from([("auth.no_user_interaction", Value::from(true))]);
    match fs.call::<_, _, String>("Mount", &(opciones,)).await {
        Ok(donde) => eprintln!("[montaje] {} montado en {donde}", ruta.as_str()),
        Err(e) => eprintln!("[montaje] no se pudo montar {}: {e}", ruta.as_str()),
    }
}

/// Monta lo que ya hay y vigila lo que llegue. No vuelve nunca.
// 0.1.0
pub async fn vigilar(c: Connection) {
    loop {
        let _ = una_vuelta(&c).await;
        // udisks2 no está o se ha reiniciado: se vuelve a intentar sin prisa.
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Mientras udisks2 siga en el bus. La suscripción va antes del recorrido:
/// al revés, lo que se enchufara entre los dos se quedaría sin montar.
// 0.1.0
async fn una_vuelta(c: &Connection) -> zbus::Result<()> {
    let om = zbus::fdo::ObjectManagerProxy::builder(c)
        .destination(UDISKS)?
        .path("/org/freedesktop/UDisks2")?
        .build()
        .await?;
    let mut nuevos = om.receive_interfaces_added().await?;
    for ruta in om.get_managed_objects().await?.keys() {
        montar_si_toca(c, ruta).await;
    }
    while let Some(senal) = nuevos.next().await {
        let Ok(args) = senal.args() else { continue };
        montar_si_toca(c, &OwnedObjectPath::from(args.object_path().clone())).await;
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn usb() -> Volumen {
        Volumen {
            con_unidad: true,
            ..Default::default()
        }
    }

    #[test]
    fn un_usb_recien_enchufado_se_monta() {
        assert!(debe_montarse(&usb()));
    }

    #[test]
    fn lo_del_sistema_y_lo_ya_montado_no() {
        assert!(!debe_montarse(&Volumen {
            sistema: true,
            ..usb()
        }));
        assert!(!debe_montarse(&Volumen {
            montado: true,
            ..usb()
        }));
        assert!(!debe_montarse(&Volumen {
            ignorar: true,
            ..usb()
        }));
    }

    /// Una imagen o un snap montados como bucle no tienen unidad física.
    #[test]
    fn un_dispositivo_de_bucle_no() {
        assert!(!debe_montarse(&Volumen::default()));
    }

    /// Si udisks recomienda montarlo aunque sea interno, se le hace caso.
    #[test]
    fn lo_que_udisks_pide_montar_se_monta() {
        assert!(debe_montarse(&Volumen {
            sistema: true,
            auto: true,
            ..usb()
        }));
    }
}

/// Contra el udisks de verdad: `BOOKOS_PROBAR_MONTAJE=/org/freedesktop/UDisks2/block_devices/sda1
/// cargo test -p bookos-system montaje_real -- --ignored`. Monta ese volumen si
/// la regla lo permite.
#[cfg(test)]
#[tokio::test]
#[ignore]
async fn montaje_real() {
    let ruta = std::env::var("BOOKOS_PROBAR_MONTAJE").expect("ruta del volumen");
    let c = Connection::system().await.expect("bus del sistema");
    let ruta = OwnedObjectPath::try_from(ruta).expect("ruta D-Bus válida");
    eprintln!(
        "decisión: {:?}",
        leer(&c, &ruta).await.map(|v| debe_montarse(&v))
    );
    montar_si_toca(&c, &ruta).await;
}
