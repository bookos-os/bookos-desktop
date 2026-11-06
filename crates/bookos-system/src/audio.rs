use bookos_system::Operation;
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

// 0.1.0
pub async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    run_with_timeout(program, args, Duration::from_secs(10)).await
}
// 0.1.0
async fn run_with_timeout(program: &str, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let stdout = child.stdout.take().ok_or("stdout no disponible")?;
    let stderr = child.stderr.take().ok_or("stderr no disponible")?;
    let reader = tokio::spawn(async move {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut stdout = stdout.take(4 * 1024 * 1024);
        let mut stderr = stderr.take(65536);
        let (a, b) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
        a.and(b).map_err(|e| e.to_string())?;
        Ok::<_, String>((out, err))
    });
    let status = match tokio::time::timeout(limit, child.wait()).await {
        Ok(r) => r.map_err(|e| e.to_string())?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            reader.abort();
            return Err(format!("{program}: tiempo de espera agotado"));
        }
    };
    let (out, err) = reader.await.map_err(|e| e.to_string())??;
    if !status.success() {
        return Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&err).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}
// 0.1.0
async fn list(kind: &str) -> Result<Value, String> {
    serde_json::from_str(&run("pactl", &["--format=json", "list", kind]).await?)
        .map_err(|e| e.to_string())
}
// 0.1.0
fn percent(item: &Value) -> u32 {
    item["volume"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
        .filter_map(|v| v["value_percent"].as_str())
        .filter_map(|s| s.trim_end_matches('%').parse().ok())
        .max()
        .unwrap_or(0)
}
// 0.1.0
fn balance(item: &Value) -> i32 {
    let channel = |name: &str| item["volume"][name]["value"].as_f64().unwrap_or(0.);
    let left = channel("front-left");
    let right = channel("front-right");
    if left.max(right) == 0. {
        0
    } else {
        ((right - left) / left.max(right) * 100.).round() as i32
    }
}
// 0.1.0
fn device(item: &Value, default: &str) -> Value {
    json!({"index":item["index"],"name":item["name"],"description":item["description"],
        "state":item["state"],"volume":percent(item),"balance":balance(item),"muted":item["mute"],"isDefault":item["name"] == default,
        "ports":item["ports"],"activePort":item["active_port"]})
}
// 0.1.0
pub async fn state() -> Result<Value, String> {
    let (sinks, sources, apps, cards, out, input) = tokio::try_join!(
        list("sinks"),
        list("sources"),
        list("sink-inputs"),
        list("cards"),
        run("pactl", &["get-default-sink"]),
        run("pactl", &["get-default-source"])
    )?;
    let sinks: Vec<_> = sinks
        .as_array()
        .ok_or("Respuesta de audio inválida")?
        .iter()
        .map(|v| device(v, &out))
        .collect();
    let sources: Vec<_> = sources
        .as_array()
        .ok_or("Respuesta de audio inválida")?
        .iter()
        .filter(|v| !v["name"].as_str().unwrap_or_default().ends_with(".monitor"))
        .map(|v| device(v, &input))
        .collect();
    let apps: Vec<_> = apps
        .as_array()
        .ok_or("Respuesta de audio inválida")?
        .iter()
        .map(|v| {
            json!({"index":v["index"],
        "name":v["properties"]["application.name"],"volume":percent(v),"muted":v["mute"]})
        })
        .collect();
    Ok(
        json!({"output":sinks.iter().find(|v| v["isDefault"] == true),"input":sources.iter().find(|v| v["isDefault"] == true),
        "sinks":sinks,"sources":sources,"defaultSink":out,"defaultSource":input,"apps":apps,"cards":cards}),
    )
}
// 0.1.0
fn target(t: &str) -> Result<(&str, &str), String> {
    match t {
        "@DEFAULT_AUDIO_SINK@" | "output" => Ok(("sink", "@DEFAULT_SINK@")),
        "@DEFAULT_AUDIO_SOURCE@" | "input" => Ok(("source", "@DEFAULT_SOURCE@")),
        _ => Err("Destino de audio desconocido".into()),
    }
}
// 0.1.0
pub async fn perform(op: &Operation) -> Result<(), String> {
    match op {
        Operation::Volume { target: t, value } | Operation::VolumePreview { target: t, value } => {
            if *value > 150 {
                return Err("Volumen fuera de rango".into());
            }
            let (kind, id) = target(t)?;
            run(
                "pactl",
                &[&format!("set-{kind}-volume"), id, &format!("{value}%")],
            )
            .await?;
        }
        Operation::VolumeStep { step } => {
            if !(-100..=100).contains(step) {
                return Err("Paso de volumen inválido".into());
            }
            run(
                "wpctl",
                &[
                    "set-volume",
                    "-l",
                    "1.0",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{}%{}", step.abs(), if *step >= 0 { "+" } else { "-" }),
                ],
            )
            .await?;
            if *step > 0 {
                run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "0"]).await?;
            }
        }
        Operation::Mute { target: t, muted } | Operation::MutePreview { target: t, muted } => {
            let (kind, id) = target(t)?;
            run(
                "pactl",
                &[
                    &format!("set-{kind}-mute"),
                    id,
                    match muted {
                        Some(true) => "1",
                        Some(false) => "0",
                        None => "toggle",
                    },
                ],
            )
            .await?;
        }
        Operation::AudioDefault { target, input } => {
            run(
                "pactl",
                &[
                    if *input {
                        "set-default-source"
                    } else {
                        "set-default-sink"
                    },
                    target,
                ],
            )
            .await?;
        }
        Operation::AppVolume { index, value } => {
            if *value > 150 {
                return Err("Volumen fuera de rango".into());
            }
            run(
                "pactl",
                &[
                    "set-sink-input-volume",
                    &index.to_string(),
                    &format!("{value}%"),
                ],
            )
            .await?;
        }
        Operation::AppMute { index, muted } => {
            run(
                "pactl",
                &[
                    "set-sink-input-mute",
                    &index.to_string(),
                    if *muted { "1" } else { "0" },
                ],
            )
            .await?;
        }
        Operation::AudioBalance { balance } => {
            if !(-100..=100).contains(balance) {
                return Err("Balance fuera de rango".into());
            }
            let s = state().await?;
            let base = s["output"]["volume"]
                .as_u64()
                .ok_or("No hay salida de audio")? as i32;
            let left = if *balance > 0 {
                base * (100 - balance) / 100
            } else {
                base
            };
            let right = if *balance < 0 {
                base * (100 + balance) / 100
            } else {
                base
            };
            run(
                "pactl",
                &[
                    "set-sink-volume",
                    "@DEFAULT_SINK@",
                    &format!("{left}%"),
                    &format!("{right}%"),
                ],
            )
            .await?;
        }
        Operation::MicrophonePower { enabled } => {
            let s = state().await?;
            for input in s["sources"].as_array().ok_or("No hay entradas de audio")? {
                run(
                    "pactl",
                    &[
                        "set-source-mute",
                        input["name"].as_str().ok_or("Entrada inválida")?,
                        if *enabled { "0" } else { "1" },
                    ],
                )
                .await?;
            }
        }
        Operation::AudioProfile { card, profile } => {
            run("pactl", &["set-card-profile", card, profile]).await?;
        }
        _ => return Err("Operación de audio no válida".into()),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exit_status_is_authoritative() {
        assert!(run("false", &[]).await.is_err());
        assert_eq!(
            run("printf", &["%s", "a:'雪;$(no)"]).await.unwrap(),
            "a:'雪;$(no)"
        );
    }
    #[tokio::test]
    async fn timeout_is_bounded() {
        let start = std::time::Instant::now();
        assert!(
            run_with_timeout("sleep", &["30"], Duration::from_millis(30))
                .await
                .unwrap_err()
                .contains("tiempo")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn balance_does_not_reduce_master_volume() {
        let v = json!({"volume":{"front-left":{"value_percent":"40%","value":40},"front-right":{"value_percent":"80%","value":80}}});
        assert_eq!(percent(&v), 80);
        assert_eq!(balance(&v), 50);
    }
    #[test]
    fn parses_structured_volume() {
        assert_eq!(
            percent(&json!({"volume":{"front-left":{"value_percent":"72%"}}})),
            72
        );
    }
}
