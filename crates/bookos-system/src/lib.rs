//! Shared wire contract and nonblocking desktop client for org.bookos.System1.
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{OnceLock, RwLock};
use tokio::sync::Notify;

pub const NAME: &str = "org.bookos.System1";
pub const PATH: &str = "/org/bookos/System1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub network: Value,
    pub bluetooth: Value,
    pub audio: Value,
    pub errors: std::collections::BTreeMap<String, String>,
}
impl Default for State {
    // 0.1.0
    fn default() -> Self {
        Self {
            version: 1,
            network: Value::Null,
            bluetooth: Value::Null,
            audio: Value::Null,
            errors: Default::default(),
        }
    }
}

/// Closed operation set; no shell command or executable crosses the interface.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    WifiPower { enabled: bool },
    WifiScan {},
    WifiConnect { ssid: String, password: String },
    WifiForget { ssid: String },
    WifiAutoconnect { ssid: String, enabled: bool },
    NetworkDisconnect { device: String },
    BluetoothPower { enabled: bool },
    BluetoothScan { enabled: bool },
    BluetoothConnect { address: String },
    BluetoothDisconnect { address: String },
    BluetoothPair { address: String },
    BluetoothForget { address: String },
    Airplane { enabled: bool },
    Volume { target: String, value: u32 },
    VolumePreview { target: String, value: u32 },
    VolumeStep { step: i32 },
    Mute { target: String, muted: Option<bool> },
    MutePreview { target: String, muted: Option<bool> },
    AudioDefault { target: String, input: bool },
    AppVolume { index: u32, value: u32 },
    AppMute { index: u32, muted: bool },
    AudioProfile { card: String, profile: String },
    AudioBalance { balance: i32 },
    MicrophonePower { enabled: bool },
}
impl Operation {
    // 0.1.0
    // modified 0.1.1 -- operación de conexión automática a una wifi
    pub fn domain(&self) -> &'static str {
        match self {
            Self::WifiPower { .. }
            | Self::WifiScan { .. }
            | Self::WifiConnect { .. }
            | Self::WifiForget { .. }
            | Self::WifiAutoconnect { .. }
            | Self::NetworkDisconnect { .. } => "network",
            Self::BluetoothPower { .. }
            | Self::BluetoothScan { .. }
            | Self::BluetoothConnect { .. }
            | Self::BluetoothDisconnect { .. }
            | Self::BluetoothPair { .. }
            | Self::BluetoothForget { .. } => "bluetooth",
            Self::Airplane { .. } => "all",
            _ => "audio",
        }
    }
}

static FEEDBACK: std::sync::Mutex<std::collections::VecDeque<(Operation, Result<(), String>)>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());
// 0.1.0
pub fn take_feedback() -> Option<(Operation, Result<(), String>)> {
    FEEDBACK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
}
static CACHE: OnceLock<RwLock<State>> = OnceLock::new();
#[derive(Default)]
struct Lane {
    queue: std::sync::Mutex<std::collections::VecDeque<Operation>>,
    ready: Notify,
}
impl Lane {
    // 0.1.0
    fn push(&self, op: Operation) -> bool {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        // Only replace adjacent absolute volume writes: never reorder toggles or steps.
        if let (Some(old), Some(target)) =
            (queue.back().and_then(volume_target), volume_target(&op))
            && old == target
        {
            queue.pop_back();
        }
        // Con la tecla mantenida llegan 25 pasos por segundo y cada uno cuesta
        // un `wpctl`, un `pactl` y un `GetState`: encolados uno a uno, el
        // volumen seguía subiendo segundos después de soltar. Sumarlos al que
        // espera no reordena nada, porque solo se juntan dos pasos seguidos.
        let op = match (queue.back(), op) {
            (Some(Operation::VolumeStep { step: previo }), Operation::VolumeStep { step }) => {
                let step = (previo + step).clamp(-100, 100);
                queue.pop_back();
                Operation::VolumeStep { step }
            }
            (_, op) => op,
        };
        if queue.len() >= 64 {
            return false;
        }
        queue.push_back(op);
        drop(queue);
        self.ready.notify_one();
        true
    }
    // 0.1.0
    async fn next(&self) -> Operation {
        loop {
            if let Some(op) = self
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front()
            {
                return op;
            }
            self.ready.notified().await;
        }
    }
}

// 0.1.0
fn volume_target(operation: &Operation) -> Option<&str> {
    match operation {
        Operation::Volume { target, .. } | Operation::VolumePreview { target, .. } => Some(target),
        _ => None,
    }
}
static SEND: OnceLock<[std::sync::Arc<Lane>; 3]> = OnceLock::new();
// 0.1.0
fn cache() -> &'static RwLock<State> {
    CACHE.get_or_init(|| RwLock::new(State::default()))
}
// 0.1.0
pub fn snapshot() -> State {
    cache().read().unwrap_or_else(|e| e.into_inner()).clone()
}
// 0.1.0
pub fn request(op: Operation) -> bool {
    SEND.get().is_some_and(|lanes| {
        let index = match op.domain() {
            "network" | "all" => 0,
            "bluetooth" => 1,
            _ => 2,
        };
        lanes[index].push(op)
    })
}
// 0.1.0
pub fn volume(input: bool) -> Option<(u8, bool)> {
    let s = snapshot();
    let v = &s.audio[if input { "input" } else { "output" }];
    Some((v["volume"].as_u64()?.min(100) as u8, v["muted"].as_bool()?))
}

/// Installs one background client. Wake is called after cache updates or errors.
/// No D-Bus operation, process wait or runtime is driven by the rendering thread.
// 0.1.0
pub fn start(wake: impl Fn() + Send + Sync + 'static) {
    let lanes = std::array::from_fn(|_| std::sync::Arc::new(Lane::default()));
    if SEND.set(lanes).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("system client runtime");
        rt.block_on(async move {
            let wake = std::sync::Arc::new(wake);
            let notifications = wake.clone();
            tokio::spawn(async move {
                loop {
                    if let Err(e) = subscribe(&*notifications).await {
                        let mut s = cache().write().unwrap_or_else(|e| e.into_inner());
                        *s = State::default();
                        s.errors.insert("service".into(), e.to_string());
                        drop(s);
                        notifications();
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            });
            let mut workers = Vec::new();
            for lane in SEND.get().unwrap() {
                let lane = lane.clone();
                let wake = wake.clone();
                workers.push(tokio::spawn(async move {
                    loop {
                        let op = lane.next().await;
                        let result = call(&op).await;
                        if result.is_ok()
                            && let Ok(c) = zbus::Connection::session().await
                            && let Ok(p) = zbus::Proxy::new(&c, NAME, PATH, NAME).await
                            && let Ok(data) = p.call::<_, _, String>("GetState", &()).await
                        {
                            update(&data, &*wake);
                        }
                        let mut queue = FEEDBACK.lock().unwrap_or_else(|e| e.into_inner());
                        if queue.len() >= 64 {
                            queue.pop_front();
                        }
                        queue.push_back((op, result));
                        drop(queue);
                        wake();
                    }
                }));
            }
            for worker in workers {
                let _ = worker.await;
            }
        });
    });
}
// 0.1.0
async fn subscribe(wake: &(dyn Fn() + Sync)) -> Result<(), zbus::Error> {
    let conn = zbus::Connection::session().await?;
    subscribe_on(conn, wake).await
}
// 0.1.0
async fn subscribe_on(conn: zbus::Connection, wake: &(dyn Fn() + Sync)) -> Result<(), zbus::Error> {
    let p = zbus::Proxy::new(&conn, NAME, PATH, NAME).await?;
    let mut changes = p.receive_signal("StateChanged").await?;
    let mut owners = p.receive_owner_changed().await?;
    let data: String = p.call("GetState", &()).await?;
    update(&data, wake);
    loop {
        tokio::select! {
            msg = changes.next() => match msg {
                Some(msg) => { let data: String = msg.body().deserialize()?; update(&data, wake); },
                None => break,
            },
            _ = owners.next() => break,
        }
    }
    Err(zbus::Error::Failure("Servicio desconectado".into()))
}
// 0.1.0
fn update(data: &str, wake: &dyn Fn()) {
    if let Ok(state) = serde_json::from_str::<State>(data) {
        *cache().write().unwrap_or_else(|e| e.into_inner()) = state;
        wake();
    }
}
// 0.1.0
pub async fn call(op: &Operation) -> Result<(), String> {
    let work = async {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| e.to_string())?;
        let p = zbus::Proxy::new(&conn, NAME, PATH, NAME)
            .await
            .map_err(|e| e.to_string())?;
        let payload = serde_json::to_string(op).map_err(|e| e.to_string())?;
        p.call::<_, _, ()>("Perform", &(payload,))
            .await
            .map_err(|e| e.to_string())
    };
    tokio::time::timeout(std::time::Duration::from_secs(110), work)
        .await
        .map_err(|_| "La operación no respondió a tiempo".to_owned())?
}

// 0.1.0
pub fn unavailable(domain: &str) -> Option<String> {
    let s = snapshot();
    s.errors
        .get("service")
        .or_else(|| s.errors.get(domain))
        .cloned()
}
// 0.1.0
pub fn audio_request(args: &[&str], notify: bool) -> bool {
    match args {
        ["set-volume", target, value] => value.parse::<f64>().ok().is_some_and(|v| {
            let target = (*target).into();
            let value = (v * 100.).round().clamp(0., 150.) as u32;
            request(if notify {
                Operation::Volume { target, value }
            } else {
                Operation::VolumePreview { target, value }
            })
        }),
        ["set-mute", target, value] => {
            let target = (*target).into();
            let muted = match *value {
                "toggle" => None,
                "1" => Some(true),
                _ => Some(false),
            };
            request(if notify {
                Operation::Mute { target, muted }
            } else {
                Operation::MutePreview { target, muted }
            })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn protocol_rejects_arbitrary_commands() {
        assert!(
            serde_json::from_value::<Operation>(json!({"operation":"run","command":"sh"})).is_err()
        );
        assert!(
            serde_json::from_value::<Operation>(json!({"operation":"wifi_scan","command":"sh"}))
                .is_err()
        );
    }
    #[tokio::test]
    async fn slider_keeps_last_value_and_command_order() {
        let lane = Lane::default();
        for value in 0..=100 {
            assert!(lane.push(Operation::VolumePreview {
                target: "output".into(),
                value
            }));
        }
        assert!(lane.push(Operation::Mute {
            target: "output".into(),
            muted: None
        }));
        assert!(lane.push(Operation::Volume {
            target: "output".into(),
            value: 42
        }));
        assert!(matches!(
            lane.next().await,
            Operation::VolumePreview { value: 100, .. }
        ));
        assert!(matches!(lane.next().await, Operation::Mute { .. }));
        assert!(matches!(
            lane.next().await,
            Operation::Volume { value: 42, .. }
        ));
    }
    #[tokio::test]
    async fn pasos_seguidos_de_volumen_se_suman() {
        let lane = Lane::default();
        for _ in 0..30 {
            assert!(lane.push(Operation::VolumeStep { step: 5 }));
        }
        assert!(lane.push(Operation::Mute {
            target: "output".into(),
            muted: None
        }));
        assert!(lane.push(Operation::VolumeStep { step: -5 }));
        assert!(matches!(
            lane.next().await,
            Operation::VolumeStep { step: 100 }
        ));
        assert!(matches!(lane.next().await, Operation::Mute { .. }));
        assert!(matches!(
            lane.next().await,
            Operation::VolumeStep { step: -5 }
        ));
    }
    #[test]
    fn names_are_data() {
        let op = Operation::WifiConnect {
            ssid: "a:'雪;$(no)".into(),
            password: "secret".into(),
        };
        let decoded: Operation =
            serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert!(matches!(decoded, Operation::WifiConnect { ssid, .. } if ssid == "a:'雪;$(no)"));
    }
}

/// Deterministic input for renderer tests; never connects to the real session.
#[cfg(feature = "test-fixtures")]
// 0.1.0
pub fn seed_test_state(state: State) {
    assert!(
        SEND.get().is_none(),
        "test data must not replace a live client"
    );
    *cache().write().unwrap_or_else(|e| e.into_inner()) = state;
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use std::{process::Stdio, sync::Arc, time::Duration};
    use tokio::io::{AsyncBufReadExt, BufReader};
    struct Mock;
    #[zbus::interface(name = "org.bookos.System1")]
    impl Mock {
        async fn get_state(&self) -> String {
            serde_json::to_string(&State {
                audio: serde_json::json!({"output":{"volume":41,"muted":false}}),
                ..Default::default()
            })
            .unwrap()
        }
        async fn perform(&self, _payload: &str) {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    #[tokio::test]
    async fn private_bus_snapshot_signal_and_owner_loss() {
        // A private bus and mock service never touch the user's hardware/session.
        let mut bus = tokio::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(bus.stdout.take().unwrap()).lines();
        let address = tokio::time::timeout(Duration::from_secs(3), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("private bus address");
        let server = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name(NAME)
            .unwrap()
            .serve_at(PATH, Mock)
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let wake = Arc::new(Notify::new());
        let callback = wake.clone();
        let subscriber =
            tokio::spawn(async move { subscribe_on(client, &move || callback.notify_one()).await });
        tokio::time::timeout(Duration::from_secs(2), wake.notified())
            .await
            .unwrap();
        assert_eq!(volume(false), Some((41, false)));
        let next = State {
            audio: serde_json::json!({"output":{"volume":73,"muted":true}}),
            ..Default::default()
        };
        server
            .emit_signal(
                None::<&str>,
                PATH,
                NAME,
                "StateChanged",
                &(serde_json::to_string(&next).unwrap(),),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), wake.notified())
            .await
            .unwrap();
        assert_eq!(volume(false), Some((73, true)));
        server.release_name(NAME).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), subscriber)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        bus.kill().await.unwrap();
        let _ = bus.wait().await;
    }
}
