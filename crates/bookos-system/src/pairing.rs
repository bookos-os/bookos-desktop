//! BlueZ agent: every authorization is routed to an explicit Settings dialog.
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, oneshot};
use zbus::{Connection, zvariant::OwnedObjectPath};

/// Petición de emparejamiento en curso: número de secuencia, lo que se enseña
/// al usuario y por dónde vuelve su respuesta.
type Pendiente = (u64, Value, oneshot::Sender<Option<String>>);

#[derive(Default)]
pub struct Broker {
    sequence: AtomicU64,
    pending: Mutex<Option<Pendiente>>,
}
impl Broker {
    // 0.1.0
    pub async fn current(&self) -> String {
        self.pending
            .lock()
            .await
            .as_ref()
            .map(|p| p.1.to_string())
            .unwrap_or("null".into())
    }
    // 0.1.0
    pub async fn answer(&self, id: u64, answer: Option<String>) -> zbus::fdo::Result<()> {
        let mut p = self.pending.lock().await;
        if p.as_ref().is_none_or(|p| p.0 != id) {
            return Err(zbus::fdo::Error::InvalidArgs("Solicitud caducada".into()));
        }
        if let Some((_, _, tx)) = p.take() {
            let _ = tx.send(answer);
        }
        Ok(())
    }
    // 0.1.0
    pub async fn clear(&self) {
        self.pending.lock().await.take();
    }
}
pub struct Agent {
    pub broker: Arc<Broker>,
    pub session: Connection,
}
impl Agent {
    // 0.1.0
    async fn prompt(
        &self,
        device: &OwnedObjectPath,
        kind: &str,
        value: String,
    ) -> zbus::fdo::Result<String> {
        let id = self.broker.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        let payload = json!({"id":id,"device":device.as_str(),"kind":kind,"value":value});
        *self.broker.pending.lock().await = Some((id, payload.clone(), tx));
        let _ = self
            .session
            .emit_signal(
                None::<&str>,
                bookos_system::PATH,
                bookos_system::NAME,
                "PairingRequested",
                &(payload.to_string(),),
            )
            .await;
        let result = tokio::time::timeout(Duration::from_secs(60), rx).await;
        self.broker.clear().await;
        let _ = self
            .session
            .emit_signal(
                None::<&str>,
                bookos_system::PATH,
                bookos_system::NAME,
                "PairingRequested",
                &("null",),
            )
            .await;
        match result {
            Ok(Ok(Some(s))) => Ok(s),
            _ => Err(zbus::fdo::Error::AccessDenied(
                "Emparejamiento cancelado".into(),
            )),
        }
    }
}
#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    // 0.1.0
    async fn release(&self) {
        self.broker.clear().await;
    }
    // 0.1.0
    async fn cancel(&self) {
        self.broker.clear().await;
    }
    // 0.1.0
    async fn request_pin_code(&self, device: OwnedObjectPath) -> zbus::fdo::Result<String> {
        self.prompt(&device, "pin", String::new()).await
    }
    // 0.1.0
    async fn request_passkey(&self, device: OwnedObjectPath) -> zbus::fdo::Result<u32> {
        let text = self.prompt(&device, "passkey", String::new()).await?;
        text.parse::<u32>()
            .ok()
            .filter(|n| *n <= 999999)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Código inválido".into()))
    }
    // 0.1.0
    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> zbus::fdo::Result<()> {
        self.prompt(&device, "confirm", format!("{passkey:06}"))
            .await
            .map(|_| ())
    }
    // 0.1.0
    async fn request_authorization(&self, device: OwnedObjectPath) -> zbus::fdo::Result<()> {
        self.prompt(&device, "authorize", String::new())
            .await
            .map(|_| ())
    }
    // 0.1.0
    async fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
    ) -> zbus::fdo::Result<()> {
        self.prompt(&device, "authorize", uuid).await.map(|_| ())
    }
    // 0.1.0
    async fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
    ) -> zbus::fdo::Result<()> {
        let payload = json!({"id":0,"device":device.as_str(),"kind":"display","value":pincode});
        self.session
            .emit_signal(
                None::<&str>,
                bookos_system::PATH,
                bookos_system::NAME,
                "PairingRequested",
                &(payload.to_string(),),
            )
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
    // 0.1.0
    async fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        _entered: u16,
    ) -> zbus::fdo::Result<()> {
        self.display_pin_code(device, format!("{passkey:06}")).await
    }
}
// 0.1.0
pub async fn register(c: &Connection) -> Result<(), String> {
    let p = zbus::Proxy::new(c, "org.bluez", "/org/bluez", "org.bluez.AgentManager1")
        .await
        .map_err(|e| e.to_string())?;
    let path = OwnedObjectPath::try_from("/org/bookos/System1/Agent").unwrap();
    match p
        .call::<_, _, ()>("RegisterAgent", &(path, "KeyboardDisplay"))
        .await
    {
        Ok(()) => Ok(()),
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.bluez.Error.AlreadyExists" =>
        {
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}
