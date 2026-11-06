mod audio;
mod bluetooth;
mod mantenimiento;
mod montaje;
mod network;
mod pairing;
use bookos_system::{NAME, Operation, PATH, State};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
struct Change {
    domain: &'static str,
    done: Option<oneshot::Sender<()>>,
}
impl From<&'static str> for Change {
    // 0.1.0
    fn from(domain: &'static str) -> Self {
        Self { domain, done: None }
    }
}
use futures_util::StreamExt;
use zbus::Connection;

fn respuesta(resultado: Result<serde_json::Value, String>) -> zbus::fdo::Result<String> {
    resultado
        .map(|v| v.to_string())
        .map_err(zbus::fdo::Error::Failed)
}

#[derive(Clone)]
struct Service {
    state: Arc<RwLock<State>>,
    changes: mpsc::Sender<Change>,
    locks: Arc<[Mutex<()>; 3]>,
    system: Connection,
    pairing: Arc<pairing::Broker>,
}
#[zbus::interface(name = "org.bookos.System1")]
impl Service {
    // 0.1.0
    async fn get_state(&self) -> String {
        serde_json::to_string(&*self.state.read().await).unwrap_or_default()
    }
    // 0.1.0
    async fn network_details(&self, kind: &str) -> zbus::fdo::Result<String> {
        network::details(&self.system, kind)
            .await
            .map(|v| v.to_string())
            .map_err(zbus::fdo::Error::Failed)
    }
    // 0.1.0
    async fn wifi_secret(&self, ssid: &str) -> zbus::fdo::Result<String> {
        network::secret(&self.system, ssid)
            .await
            .map_err(zbus::fdo::Error::Failed)
    }
    // 0.1.0
    async fn get_pairing(&self) -> String {
        self.pairing.current().await
    }
    // 0.1.0
    async fn answer_pairing(
        &self,
        id: u64,
        accepted: bool,
        value: String,
    ) -> zbus::fdo::Result<()> {
        self.pairing.answer(id, accepted.then_some(value)).await
    }
    async fn maintenance_overview(&self) -> zbus::fdo::Result<String> {
        respuesta(mantenimiento::resumen().await)
    }
    async fn maintenance_sizes(&self) -> zbus::fdo::Result<String> {
        respuesta(mantenimiento::tamanos().await)
    }
    async fn maintenance_scan(&self) -> zbus::fdo::Result<String> {
        respuesta(mantenimiento::escanear().await)
    }
    async fn maintenance_clean(&self, payload: &str) -> zbus::fdo::Result<String> {
        respuesta(mantenimiento::limpiar(payload).await)
    }
    async fn maintenance_trash(&self, payload: &str) -> zbus::fdo::Result<String> {
        respuesta(mantenimiento::a_papelera(payload).await)
    }
    // 0.1.0
    async fn perform(&self, payload: &str) -> zbus::fdo::Result<()> {
        if payload.len() > 65536 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Petición demasiado grande".into(),
            ));
        }
        let op: Operation = serde_json::from_str(payload)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let index = match op.domain() {
            "network" | "all" => 0,
            "bluetooth" => 1,
            _ => 2,
        };
        let _guard = self.locks[index].lock().await;
        let _radio_guard = if op.domain() == "all" {
            Some(self.locks[1].lock().await)
        } else {
            None
        };
        let work = async {
            match op.domain() {
                "network" => network::perform(&self.system, &op).await,
                "bluetooth" => {
                    if matches!(op, Operation::BluetoothPair { .. }) {
                        pairing::register(&self.system).await?;
                    }
                    bluetooth::perform(&self.system, &op).await
                }
                "audio" => audio::perform(&op).await,
                _ => {
                    let Operation::Airplane { enabled } = op else {
                        unreachable!()
                    };
                    // rfkill handles all radios, including adapters unavailable to BlueZ.
                    audio::run(
                        "rfkill",
                        &[if enabled { "block" } else { "unblock" }, "all"],
                    )
                    .await?;
                    Ok(())
                }
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(90), work)
            .await
            .map_err(|_| "Tiempo de espera agotado".to_owned())
            .and_then(|r| r);
        let (tx, rx) = oneshot::channel();
        let _ = self
            .changes
            .send(Change {
                domain: op.domain(),
                done: Some(tx),
            })
            .await;
        if tokio::time::timeout(Duration::from_secs(15), rx)
            .await
            .ok()
            .and_then(Result::ok)
            .is_none()
            && result.is_ok()
        {
            return Err(zbus::fdo::Error::Failed(
                "Operación ejecutada; no se pudo confirmar el estado actualizado".into(),
            ));
        }
        if result.is_ok() {
            let state = self.state.read().await;
            let error = if op.domain() == "all" {
                state
                    .errors
                    .get("network")
                    .or_else(|| state.errors.get("bluetooth"))
            } else {
                state.errors.get(op.domain())
            };
            if let Some(error) = error {
                return Err(zbus::fdo::Error::Failed(format!(
                    "No se pudo confirmar el estado: {error}"
                )));
            }
        }
        result.map_err(zbus::fdo::Error::Failed)
    }
    #[zbus(signal)]
    // 0.1.0
    async fn state_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        state: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    // 0.1.0
    async fn pairing_requested(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        request: &str,
    ) -> zbus::Result<()>;
}

#[tokio::main]
// 0.1.0
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let system = Connection::system().await?;
    let (changes, mut events) = mpsc::channel(32);
    let state = Arc::new(RwLock::new(State::default()));
    let pairing = Arc::new(pairing::Broker::default());
    let service = Service {
        state: state.clone(),
        changes: changes.clone(),
        locks: Arc::new([Mutex::new(()), Mutex::new(()), Mutex::new(())]),
        system: system.clone(),
        pairing: pairing.clone(),
    };
    let conn = zbus::connection::Builder::session()?
        .name(NAME)?
        .serve_at(PATH, service)?
        .build()
        .await?;
    system
        .object_server()
        .at(
            "/org/bookos/System1/Agent",
            pairing::Agent {
                broker: pairing,
                session: conn.clone(),
            },
        )
        .await?;
    for (name, domain) in [
        ("org.freedesktop.NetworkManager", "network"),
        ("org.bluez", "bluetooth"),
    ] {
        tokio::spawn(watch_bus(system.clone(), name, domain, changes.clone()));
        tokio::spawn(watch_owner(system.clone(), name, domain, changes.clone()));
    }
    tokio::spawn(watch_audio(changes.clone()));
    tokio::spawn(montaje::vigilar(system.clone()));
    changes.send("all".into()).await?;
    while let Some(domain) = events.recv().await {
        // Batch notifications from the same hardware transaction.
        tokio::time::sleep(Duration::from_millis(40)).await;
        let mut domains = std::collections::HashSet::from([domain.domain]);
        let mut replies = Vec::new();
        if let Some(tx) = domain.done {
            replies.push(tx);
        }
        while let Ok(d) = events.try_recv() {
            domains.insert(d.domain);
            if let Some(tx) = d.done {
                replies.push(tx);
            }
        }
        let all = domains.contains("all");
        let update = |d: &'static str| {
            let system = &system;
            let wanted = all || domains.contains(d);
            async move {
                if !wanted {
                    return None;
                }
                let result = tokio::time::timeout(Duration::from_secs(12), async {
                    match d {
                        "network" => network::state(system).await,
                        "bluetooth" => bluetooth::state(system).await,
                        _ => audio::state().await,
                    }
                })
                .await
                .map_err(|_| "Servicio sin respuesta".to_owned())
                .and_then(|r| r);
                Some((d, result))
            }
        };
        let (network, bluetooth, audio) =
            tokio::join!(update("network"), update("bluetooth"), update("audio"));
        let mut next = state.read().await.clone();
        for (d, result) in [network, bluetooth, audio].into_iter().flatten() {
            let data = match result {
                Ok(data) => {
                    next.errors.remove(d);
                    data
                }
                Err(e) => {
                    next.errors.insert(d.into(), e);
                    serde_json::Value::Null
                }
            };
            match d {
                "network" => next.network = data,
                "bluetooth" => next.bluetooth = data,
                _ => next.audio = data,
            }
        }
        if *state.read().await != next {
            let data = serde_json::to_string(&next)?;
            *state.write().await = next;
            conn.emit_signal(None::<&str>, PATH, NAME, "StateChanged", &(data,))
                .await?;
        }
        for tx in replies {
            let _ = tx.send(());
        }
    }
    Ok(())
}
// 0.1.0
async fn watch_bus(
    c: Connection,
    name: &'static str,
    domain: &'static str,
    tx: mpsc::Sender<Change>,
) {
    loop {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(name)
            .unwrap()
            .build();
        if let Ok(mut stream) = zbus::MessageStream::for_match_rule(rule, &c, Some(128)).await {
            let _ = tx.send(domain.into()).await;
            while stream.next().await.is_some() {
                let _ = tx.try_send(domain.into());
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
// 0.1.0
async fn watch_audio(tx: mpsc::Sender<Change>) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    loop {
        if let Ok(mut child) = tokio::process::Command::new("pactl")
            .arg("subscribe")
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            let _ = tx.send("audio".into()).await;
            if let Some(out) = child.stdout.take() {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if audio_event(&line) {
                        let _ = tx.try_send("audio".into());
                    }
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let _ = tx.send("audio".into()).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

// 0.1.0
async fn watch_owner(
    c: Connection,
    name: &'static str,
    domain: &'static str,
    tx: mpsc::Sender<Change>,
) {
    // NetworkManager y BlueZ pueden aparecer después del servicio (arranque
    // lento, reanudación o reinicio). El watcher anterior solo intentaba crear
    // el proxy una vez y se quedaba muerto para el resto de la sesión.
    loop {
        if let Ok(p) = zbus::Proxy::new(&c, name, "/", name).await
            && let Ok(mut owners) = p.receive_owner_changed().await
        {
            // Al encontrar el servicio por primera vez, fuerza una lectura
            // inmediata; después solo despierta al shell cuando cambia el
            // propietario. Si aún no existe, no generamos ticks inútiles.
            let _ = tx.send(domain.into()).await;
            while owners.next().await.is_some() {
                let _ = tx.send(domain.into()).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

// 0.1.0
fn audio_event(line: &str) -> bool {
    [
        " on sink #",
        " on source #",
        " on sink-input #",
        " on card #",
        " on server #",
    ]
    .iter()
    .any(|kind| line.contains(kind))
}
#[cfg(test)]
mod tests {
    #[test]
    fn queries_do_not_trigger_new_queries() {
        assert!(!super::audio_event("Event 'new' on client #123"));
        assert!(!super::audio_event("Event 'remove' on client #123"));
        assert!(super::audio_event("Event 'change' on sink #1"));
        assert!(super::audio_event("Event 'remove' on sink-input #2"));
    }
}
