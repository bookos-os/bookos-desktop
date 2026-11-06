use bookos_system::Operation;
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};
type Settings = HashMap<String, HashMap<String, OwnedValue>>;
const NAME: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
// 0.1.0
async fn proxy<'a>(c: &'a Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(c, NAME, path, iface)
        .await
        .map_err(|e| e.to_string())
}
// 0.1.0
async fn devices(c: &Connection) -> Result<Vec<OwnedObjectPath>, String> {
    proxy(c, ROOT, NAME)
        .await?
        .call("GetDevices", &())
        .await
        .map_err(|e| e.to_string())
}
// 0.1.0
async fn wifi_device(c: &Connection) -> Result<OwnedObjectPath, String> {
    for d in devices(c).await? {
        let p = proxy(c, d.as_str(), "org.freedesktop.NetworkManager.Device").await?;
        if p.get_property::<u32>("DeviceType")
            .await
            .map_err(|e| e.to_string())?
            == 2
        {
            return Ok(d);
        }
    }
    Err("No hay adaptador Wi-Fi".into())
}
// 0.1.0
// modified 0.1.1 -- marca qué redes están guardadas y si se conectan solas
pub async fn state(c: &Connection) -> Result<Json, String> {
    let manager = proxy(c, ROOT, NAME).await?;
    let enabled: bool = manager
        .get_property("WirelessEnabled")
        .await
        .map_err(|e| e.to_string())?;
    let hardware: bool = manager
        .get_property("WirelessHardwareEnabled")
        .await
        .map_err(|e| e.to_string())?;
    let mut networks = Vec::new();
    let mut ethernet = json!({"present":false,"connected":false});
    let mut ssid = String::new();
    // Una lectura por refresco y no una por red: con 9 perfiles son ~3,4 ms
    // (medido), poco al lado de las 5 propiedades que se piden por cada AP.
    let guardadas: HashMap<Vec<u8>, bool> =
        wifi_profiles(c).await?.into_iter().fold(HashMap::new(), |mut m, (ssid, _, s)| {
            *m.entry(ssid).or_default() |= autoconnect(&s);
            m
        });
    for d in devices(c).await? {
        let device = proxy(c, d.as_str(), "org.freedesktop.NetworkManager.Device").await?;
        let kind: u32 = device
            .get_property("DeviceType")
            .await
            .map_err(|e| e.to_string())?;
        let status: u32 = device
            .get_property("State")
            .await
            .map_err(|e| e.to_string())?;
        if kind == 1 {
            ethernet = json!({"present":true,"connected":status == 100,"device":d.as_str(),
                "interface":device.get_property::<String>("Interface").await.unwrap_or_default()});
        }
        if kind != 2 {
            continue;
        }
        let w = proxy(
            c,
            d.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
        )
        .await?;
        let active: OwnedObjectPath = w
            .get_property("ActiveAccessPoint")
            .await
            .map_err(|e| e.to_string())?;
        let aps: Vec<OwnedObjectPath> = w
            .call("GetAllAccessPoints", &())
            .await
            .map_err(|e| e.to_string())?;
        for ap in aps {
            let a = proxy(c, ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint").await?;
            // Access points can disappear between enumeration and the property read.
            let Ok(bytes) = a.get_property::<Vec<u8>>("Ssid").await else {
                continue;
            };
            let name = String::from_utf8_lossy(&bytes).to_string();
            if name.is_empty() {
                continue;
            }
            let strength = a.get_property::<u8>("Strength").await.unwrap_or(0);
            let freq = a.get_property::<u32>("Frequency").await.unwrap_or(0);
            let flags = a.get_property::<u32>("Flags").await.unwrap_or(0);
            let wpa = a.get_property::<u32>("WpaFlags").await.unwrap_or(0);
            let rsn = a.get_property::<u32>("RsnFlags").await.unwrap_or(0);
            let connected = ap == active && status == 100;
            if connected {
                ssid = name.clone();
            }
            let guardada = guardadas.get(&bytes);
            networks.push(json!({"id":ap.as_str(),"device":d.as_str(),"ssid":name,
                "signal":strength,"active":connected,"saved":guardada.is_some(),"autoconnect":guardada.copied().unwrap_or(false),"security":if flags & 1 == 0 { "" } else if rsn != 0 { "WPA2" } else if wpa != 0 { "WPA" } else { "WEP" },
                "band":if freq >= 5925 { "6G" } else if freq >= 3000 { "5G" } else { "2.4G" }}));
        }
    }
    networks.sort_by_key(|n| {
        (
            std::cmp::Reverse(n["active"].as_bool().unwrap_or(false)),
            std::cmp::Reverse(n["signal"].as_u64().unwrap_or(0)),
        )
    });
    // Keep the strongest (or connected) BSSID for each displayed SSID.
    let mut seen = std::collections::HashSet::new();
    networks.retain(|n| seen.insert(n["ssid"].as_str().unwrap_or_default().to_owned()));
    Ok(
        json!({"enabled":enabled,"hardwareEnabled":hardware,"ssid":ssid,"networks":networks,"ethernet":ethernet}),
    )
}
/// Perfiles Wi-Fi guardados: SSID, ruta y ajustes (sin secretos).
// 0.1.1
async fn wifi_profiles(c: &Connection) -> Result<Vec<(Vec<u8>, OwnedObjectPath, Settings)>, String> {
    let settings = proxy(
        c,
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .await?;
    let paths: Vec<OwnedObjectPath> = settings
        .call("ListConnections", &())
        .await
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for path in paths {
        let p = proxy(
            c,
            path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await?;
        let s: Settings = p
            .call("GetSettings", &())
            .await
            .map_err(|e| e.to_string())?;
        let ssid = s
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok());
        if let Some(ssid) = ssid {
            result.push((ssid, path, s));
        }
    }
    Ok(result)
}
// 0.1.0
// modified 0.1.1 -- sale de wifi_profiles, compartida
async fn profiles(c: &Connection, ssid: &str) -> Result<Vec<OwnedObjectPath>, String> {
    Ok(wifi_profiles(c)
        .await?
        .into_iter()
        .filter(|(s, _, _)| s == ssid.as_bytes())
        .map(|(_, path, _)| path)
        .collect())
}
/// NetworkManager no manda las propiedades que están en su valor por
/// defecto, y `connection.autoconnect` lo tiene a `true`.
// 0.1.1
fn autoconnect(s: &Settings) -> bool {
    s.get("connection")
        .and_then(|c| c.get("autoconnect"))
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(true)
}
// 0.1.0
// modified 0.1.1 -- activar o desactivar la conexión automática a una wifi guardada
pub async fn perform(c: &Connection, op: &Operation) -> Result<(), String> {
    let manager = proxy(c, ROOT, NAME).await?;
    match op {
        Operation::WifiPower { enabled } => manager
            .set_property("WirelessEnabled", enabled)
            .await
            .map_err(|e| e.to_string()),
        Operation::WifiScan {} => {
            let d = wifi_device(c).await?;
            proxy(
                c,
                d.as_str(),
                "org.freedesktop.NetworkManager.Device.Wireless",
            )
            .await?
            .call::<_, _, ()>("RequestScan", &(HashMap::<String, OwnedValue>::new(),))
            .await
            .map_err(|e| e.to_string())
        }
        Operation::NetworkDisconnect { device } => {
            if !devices(c).await?.iter().any(|d| d.as_str() == device) {
                return Err("Dispositivo desconocido".into());
            }
            proxy(c, device, "org.freedesktop.NetworkManager.Device")
                .await?
                .call::<_, _, ()>("Disconnect", &())
                .await
                .map_err(|e| e.to_string())
        }
        Operation::WifiForget { ssid } => {
            for path in profiles(c, ssid).await? {
                proxy(
                    c,
                    path.as_str(),
                    "org.freedesktop.NetworkManager.Settings.Connection",
                )
                .await?
                .call::<_, _, ()>("Delete", &())
                .await
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        // `Update` con lo que devuelve `GetSettings`, que no lleva secretos, no
        // borra la contraseña guardada: comprobado con NetworkManager 1.56 en
        // un perfil de prueba (la psk seguía tras pasar autoconnect a «no»).
        Operation::WifiAutoconnect { ssid, enabled } => {
            let perfiles: Vec<_> = wifi_profiles(c)
                .await?
                .into_iter()
                .filter(|(s, _, _)| s == ssid.as_bytes())
                .collect();
            if perfiles.is_empty() {
                return Err("No hay perfil guardado para esa red".into());
            }
            for (_, path, mut settings) in perfiles {
                settings.entry("connection".into()).or_default().insert(
                    "autoconnect".into(),
                    Value::from(*enabled).try_to_owned().map_err(|e| e.to_string())?,
                );
                proxy(c, path.as_str(), "org.freedesktop.NetworkManager.Settings.Connection")
                    .await?
                    .call::<_, _, ()>("Update", &(settings,))
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Operation::WifiConnect { ssid, password } => {
            let s = state(c).await?;
            let ap = s["networks"]
                .as_array()
                .and_then(|ns| ns.iter().find(|n| n["ssid"] == ssid.as_str()))
                .ok_or("Red fuera de alcance")?;
            let d = OwnedObjectPath::try_from(ap["device"].as_str().ok_or("Dispositivo inválido")?)
                .map_err(|e| e.to_string())?;
            let a = OwnedObjectPath::try_from(ap["id"].as_str().ok_or("Red inválida")?)
                .map_err(|e| e.to_string())?;
            let saved = profiles(c, ssid).await?;
            let active: OwnedObjectPath = if !saved.is_empty() {
                if !password.is_empty() {
                    let profile = proxy(
                        c,
                        saved[0].as_str(),
                        "org.freedesktop.NetworkManager.Settings.Connection",
                    )
                    .await?;
                    let mut settings: Settings = profile
                        .call("GetSettings", &())
                        .await
                        .map_err(|e| e.to_string())?;
                    settings
                        .entry("802-11-wireless-security".into())
                        .or_default()
                        .insert(
                            "psk".into(),
                            Value::from(password.as_str())
                                .try_to_owned()
                                .map_err(|e| e.to_string())?,
                        );
                    profile
                        .call::<_, _, ()>("Update", &(settings,))
                        .await
                        .map_err(|e| e.to_string())?;
                }
                manager
                    .call("ActivateConnection", &(&saved[0], &d, &a))
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
                settings.insert(
                    "connection",
                    HashMap::from([
                        ("id", Value::from(ssid.as_str())),
                        ("type", Value::from("802-11-wireless")),
                    ]),
                );
                settings.insert(
                    "802-11-wireless",
                    HashMap::from([("ssid", Value::from(ssid.as_bytes().to_vec()))]),
                );
                if !password.is_empty() {
                    settings.insert(
                        "802-11-wireless-security",
                        HashMap::from([
                            ("key-mgmt", Value::from("wpa-psk")),
                            ("psk", Value::from(password.as_str())),
                        ]),
                    );
                }
                let (_, connection): (OwnedObjectPath, OwnedObjectPath) = manager
                    .call("AddAndActivateConnection", &(settings, &d, &a))
                    .await
                    .map_err(|e| e.to_string())?;
                connection
            };
            let connection = proxy(
                c,
                active.as_str(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )
            .await?;
            let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
                loop {
                    let status: u32 = connection
                        .get_property("State")
                        .await
                        .map_err(|e| e.to_string())?;
                    match status {
                        2 => return Ok(()),
                        3 | 4 => return Err("No se pudo conectar a la red".to_owned()),
                        _ => {}
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            })
            .await;
            match result {
                Ok(result) => result,
                Err(_) => {
                    let _ = manager
                        .call::<_, _, ()>("DeactivateConnection", &(&active,))
                        .await;
                    Err("La conexión Wi-Fi no respondió a tiempo".into())
                }
            }
        }
        _ => Err("Operación de red no válida".into()),
    }
}

// 0.1.0
pub async fn details(c: &Connection, kind: &str) -> Result<Json, String> {
    let wanted = match kind {
        "wifi" => 2,
        "ethernet" => 1,
        _ => return Err("Tipo de red inválido".into()),
    };
    for d in devices(c).await? {
        let p = proxy(c, d.as_str(), "org.freedesktop.NetworkManager.Device").await?;
        if p.get_property::<u32>("DeviceType")
            .await
            .map_err(|e| e.to_string())?
            != wanted
        {
            continue;
        }
        let iface: String = p
            .get_property("Interface")
            .await
            .map_err(|e| e.to_string())?;
        let status: u32 = p.get_property("State").await.map_err(|e| e.to_string())?;
        let mac = p
            .get_property::<String>("HwAddress")
            .await
            .unwrap_or_default();
        let mtu = p.get_property::<u32>("Mtu").await.unwrap_or(0);
        let mut result = json!({"present":true,"iface":iface,"device":d.as_str(),"mac":mac,"mtu":mtu.to_string(),
            "state":if status == 100 {"connected"} else {"disconnected"},"ip":"","ip6":"","gateway":"","dns":"","prefix":""});
        for (property, interface, key) in [
            (
                "Ip4Config",
                "org.freedesktop.NetworkManager.IP4Config",
                "ip",
            ),
            (
                "Ip6Config",
                "org.freedesktop.NetworkManager.IP6Config",
                "ip6",
            ),
        ] {
            let path: OwnedObjectPath =
                p.get_property(property).await.map_err(|e| e.to_string())?;
            if path.as_str() == "/" {
                continue;
            }
            let config = proxy(c, path.as_str(), interface).await?;
            let addresses: Vec<HashMap<String, OwnedValue>> = config
                .get_property("AddressData")
                .await
                .map_err(|e| e.to_string())?;
            if let Some(a) = addresses.first() {
                result[key] = json!(
                    a.get("address")
                        .and_then(|v| <&str>::try_from(v).ok())
                        .unwrap_or_default()
                );
                if key == "ip" {
                    result["prefix"] = json!(
                        a.get("prefix")
                            .and_then(|v| u32::try_from(v).ok())
                            .map(|v| v.to_string())
                            .unwrap_or_default()
                    );
                    result["gateway"] = json!(
                        config
                            .get_property::<String>("Gateway")
                            .await
                            .unwrap_or_default()
                    );
                    let dns: Vec<HashMap<String, OwnedValue>> = config
                        .get_property("NameserverData")
                        .await
                        .unwrap_or_default();
                    result["dns"] = json!(
                        dns.iter()
                            .filter_map(|a| a.get("address").and_then(|v| <&str>::try_from(v).ok()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }
        return Ok(result);
    }
    Err("No hay dispositivo de red".into())
}
// 0.1.0
pub async fn secret(c: &Connection, ssid: &str) -> Result<String, String> {
    let paths = profiles(c, ssid).await?;
    let path = paths.first().ok_or("No hay perfil guardado para esa red")?;
    let p = proxy(
        c,
        path.as_str(),
        "org.freedesktop.NetworkManager.Settings.Connection",
    )
    .await?;
    let settings: Settings = p
        .call("GetSecrets", &("802-11-wireless-security",))
        .await
        .map_err(|e| e.to_string())?;
    settings
        .get("802-11-wireless-security")
        .and_then(|s| s.get("psk"))
        .and_then(|v| <&str>::try_from(v).ok())
        .map(str::to_owned)
        .ok_or("No hay contraseña disponible".into())
}
