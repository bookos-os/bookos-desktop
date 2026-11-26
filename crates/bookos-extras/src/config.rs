//! Configuración persistente de `bookos-extras` (`~/.config/bookos/extras.toml`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Interruptor maestro de la navegación táctil.
    pub activo: bool,
    /// Ancho de la franja de borde en píxeles lógicos.
    pub margen_borde: f64,
    /// Habilita el gesto «Atrás» desde los bordes izquierdo y derecho.
    pub gesto_atras: bool,
    /// Habilita el deslizamiento horizontal en el borde inferior para cambiar de escritorio.
    pub gesto_inferior_horizontal: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            activo: true,
            margen_borde: 24.0,
            gesto_atras: true,
            gesto_inferior_horizontal: true,
        }
    }
}

pub fn ruta_config() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg).join("bookos/extras.toml");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".config/bookos/extras.toml")
}

pub fn ruta_pid() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(dir).join("bookos-extras.pid")
}

pub fn cargar() -> Config {
    let ruta = ruta_config();
    match std::fs::read_to_string(&ruta) {
        Ok(texto) => toml::from_str(&texto).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn guardar(cfg: &Config) -> anyhow::Result<()> {
    let ruta = ruta_config();
    if let Some(padre) = ruta.parent() {
        std::fs::create_dir_all(padre)?;
    }
    let texto = toml::to_string_pretty(cfg)?;
    std::fs::write(&ruta, texto)?;
    Ok(())
}

/// Avisa al demonio en ejecución (si existe) para que recargue `extras.toml`.
pub fn notificar_recarga() {
    let Ok(texto) = std::fs::read_to_string(ruta_pid()) else {
        return;
    };
    let Ok(pid) = texto.trim().parse::<i32>() else {
        return;
    };
    if pid > 1 {
        // SAFETY: kill con SIGUSR1 sobre el PID registrado del propio usuario.
        unsafe {
            libc::kill(pid, libc::SIGUSR1);
        }
    }
}
