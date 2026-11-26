//! Protocolo IPC compartido con el hook pasivo de `bookos-comp`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Borde {
    Arriba,
    Abajo,
    Izquierda,
    Derecha,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccionNotifDto {
    pub clave: String,
    pub etiqueta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagenNotifDto {
    pub ancho: u32,
    pub alto: u32,
    pub rgba_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificacionDto {
    pub id: u32,
    pub app: String,
    pub resumen: String,
    pub cuerpo: String,
    pub critica: bool,
    pub leida: bool,
    pub progreso: Option<u8>,
    pub segundos: u64,
    pub acciones: Vec<AccionNotifDto>,
    #[serde(default)]
    pub tiene_default: bool,
    #[serde(default)]
    pub es_captura: bool,
    #[serde(default)]
    pub imagen: Option<ImagenNotifDto>,
}

/// Eventos recibidos desde `bookos-comp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Evento {
    Pantalla {
        ancho: f64,
        alto: f64,
        escala: f64,
        oscuro: bool,
        #[serde(default)]
        efectos_reducidos: bool,
    },
    ToqueInicio {
        slot: u32,
        x: f64,
        y: f64,
        ancho: f64,
        alto: f64,
        borde: Borde,
        #[serde(default = "verdadero")]
        panel_visible: bool,
        #[serde(default)]
        exposicion_abierta: bool,
        tiempo: u32,
    },
    ToqueMueve {
        slot: u32,
        x: f64,
        y: f64,
        tiempo: u32,
    },
    ToqueFin {
        slot: u32,
        tiempo: u32,
    },
    ToqueCancelar {
        slot: u32,
        tiempo: u32,
    },
    ToqueFuera,
    Notificaciones {
        lista: Vec<NotificacionDto>,
        #[serde(default)]
        no_molestar: bool,
    },
}

/// Peticiones enviadas hacia `bookos-comp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Peticion {
    Config {
        activo: bool,
        margen_borde: f64,
        gesto_inferior_horizontal: bool,
    },
    MenuBookos,
    CentroControl,
    MostrarPanel,
    VistaInicio,
    VistaProgreso {
        progreso: f32,
    },
    VistaSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    ExposicionProgreso {
        progreso: f32,
        #[serde(default)]
        desplazamiento_x: f32,
    },
    ExposicionSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    ExposicionEscritorioProgreso {
        progreso: f32,
        #[serde(default)]
        desplazamiento_x: f32,
    },
    ExposicionEscritorioSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    DespejarEscritorio,
    DespejeProgreso {
        progreso: f32,
    },
    DespejeSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    EscritorioInicio,
    EscritorioProgreso {
        progreso: f32,
    },
    EscritorioSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    Atras,
    Adelante,
    PedirNotificaciones,
    CerrarNotificacion {
        id: u32,
    },
    LimpiarNotificaciones,
    InvocarAccionNotificacion {
        id: u32,
        clave: String,
    },
    MarcarNotificacionesLeidas,
    CortinaAbierta,
    AlternarNoMolestar,
    AbrirAjustesNotificaciones,
}

fn verdadero() -> bool {
    true
}

/// Resuelve la ruta del socket de `bookos-comp`.
pub fn ruta_socket() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("BOOKOS_EXTRAS_SOCK")
        && !custom.is_empty()
    {
        return Some(PathBuf::from(custom));
    }
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if let Ok(display) = std::env::var("WAYLAND_DISPLAY")
        && !display.is_empty()
    {
        return Some(dir.join(format!("bookos-extras-{display}.sock")));
    }
    Some(dir.join("bookos-extras-wayland-0.sock"))
}
