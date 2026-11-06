use bookos_system::Operation;
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use zbus::{Connection, Proxy, zvariant::OwnedObjectPath};
type Objects =
    HashMap<OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>>;
// 0.1.0
async fn objects(c: &Connection) -> Result<Objects, String> {
    Proxy::new(c, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager")
        .await
        .map_err(|e| e.to_string())?
        .call("GetManagedObjects", &())
        .await
        .map_err(|e| e.to_string())
}
// 0.1.0
async fn proxy<'a>(c: &'a Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(c, "org.bluez", path, iface)
        .await
        .map_err(|e| e.to_string())
}
// 0.1.0
pub async fn state(c: &Connection) -> Result<Json, String> {
    let mut enabled = false;
    let mut present = false;
    let mut devices = Vec::new();
    for (path, interfaces) in objects(c).await? {
        if let Some(a) = interfaces.get("org.bluez.Adapter1") {
            present = true;
            enabled |= a
                .get("Powered")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false);
        }
        if let Some(d) = interfaces.get("org.bluez.Device1") {
            let string = |k| {
                d.get(k)
                    .and_then(|v| <&str>::try_from(v).ok())
                    .unwrap_or_default()
                    .to_owned()
            };
            let boolean = |k| {
                d.get(k)
                    .and_then(|v| bool::try_from(v).ok())
                    .unwrap_or(false)
            };
            let battery = interfaces
                .get("org.bluez.Battery1")
                .and_then(|b| b.get("Percentage"))
                .and_then(|v| u8::try_from(v).ok());
            devices.push(json!({"id":path.as_str(),"mac":string("Address"),"name":string("Alias"),
                "connected":boolean("Connected"),"paired":boolean("Paired"),"trusted":boolean("Trusted"),"icon":string("Icon"),"battery":battery}));
        }
    }
    devices.sort_by_key(|d| {
        (
            std::cmp::Reverse(d["connected"].as_bool()),
            d["name"].as_str().unwrap_or_default().to_owned(),
        )
    });
    Ok(json!({"present":present,"enabled":enabled,"devices":devices}))
}
// 0.1.0
pub async fn perform(c: &Connection, op: &Operation) -> Result<(), String> {
    match op {
        Operation::BluetoothPower { enabled } | Operation::BluetoothScan { enabled } => {
            let mut found = false;
            for (path, interfaces) in objects(c).await? {
                if !interfaces.contains_key("org.bluez.Adapter1") {
                    continue;
                }
                found = true;
                let p = proxy(c, path.as_str(), "org.bluez.Adapter1").await?;
                if matches!(op, Operation::BluetoothPower { .. }) {
                    p.set_property("Powered", enabled)
                        .await
                        .map_err(|e| e.to_string())?;
                } else {
                    p.call::<_, _, ()>(
                        if *enabled {
                            "StartDiscovery"
                        } else {
                            "StopDiscovery"
                        },
                        &(),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                }
            }
            if found {
                Ok(())
            } else {
                Err("No hay adaptador Bluetooth".into())
            }
        }
        Operation::BluetoothConnect { address }
        | Operation::BluetoothDisconnect { address }
        | Operation::BluetoothPair { address }
        | Operation::BluetoothForget { address } => {
            let s = state(c).await?;
            let d = s["devices"]
                .as_array()
                .and_then(|ds| {
                    ds.iter().find(|d| {
                        d["mac"]
                            .as_str()
                            .is_some_and(|a| a.eq_ignore_ascii_case(address))
                    })
                })
                .ok_or("Dispositivo Bluetooth desconocido")?;
            let path = d["id"].as_str().ok_or("Dispositivo inválido")?;
            let p = proxy(c, path, "org.bluez.Device1").await?;
            if matches!(op, Operation::BluetoothForget { .. }) {
                let adapter: OwnedObjectPath =
                    p.get_property("Adapter").await.map_err(|e| e.to_string())?;
                let path = OwnedObjectPath::try_from(path).map_err(|e| e.to_string())?;
                return proxy(c, adapter.as_str(), "org.bluez.Adapter1")
                    .await?
                    .call::<_, _, ()>("RemoveDevice", &(path,))
                    .await
                    .map_err(|e| e.to_string());
            }
            let method = match op {
                Operation::BluetoothConnect { .. } => "Connect",
                Operation::BluetoothDisconnect { .. } => "Disconnect",
                _ => "Pair",
            };
            p.call::<_, _, ()>(method, &())
                .await
                .map_err(|e| e.to_string())
        }
        _ => Err("Operación Bluetooth no válida".into()),
    }
}
