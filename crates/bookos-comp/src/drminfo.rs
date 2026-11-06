//! Diagnóstico de KMS en frío (`bookos-comp --drm-info`).
//!
//! Enumera GPU, conectores y modos **sin** abrir sesión de libseat ni pedir
//! DRM master, así que se puede ejecutar dentro de la sesión de KDE sin
//! molestarla. Sirve para comprobar que el hardware es el que creemos antes de
//! arriesgarse a arrancar en un TTY, y para tener a mano los modos reales
//! cuando el backend udev no muestre nada.

use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;

use smithay::reexports::drm::Device as BasicDevice;
use smithay::reexports::drm::control::{Device as ControlDevice, connector};

/// Envoltorio mínimo: los traits de `drm` solo piden el descriptor.
struct Card(File);

impl AsFd for Card {
    // 0.1.0
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl BasicDevice for Card {}
impl ControlDevice for Card {}

// 0.1.0
pub fn run() -> anyhow::Result<()> {
    let mut encontradas = 0;

    for entrada in std::fs::read_dir("/dev/dri")? {
        let ruta = entrada?.path();
        let nombre = ruta.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !nombre.starts_with("card") {
            continue;
        }
        encontradas += 1;
        if let Err(err) = describir(&ruta) {
            println!("{}: no se pudo consultar ({err})", ruta.display());
        }
    }

    if encontradas == 0 {
        anyhow::bail!("no hay ningún /dev/dri/card*: ¿falta el driver de vídeo?");
    }
    Ok(())
}

// 0.1.0
fn describir(ruta: &Path) -> anyhow::Result<()> {
    // Solo lectura y sin master: enumerar recursos no requiere control.
    let card = Card(File::open(ruta)?);
    let recursos = card.resource_handles()?;

    println!("\n=== {} ===", ruta.display());
    if let Ok(version) = card.get_driver() {
        println!(
            "  driver: {} ({})",
            version.name().to_string_lossy(),
            version.description().to_string_lossy()
        );
    }
    println!(
        "  crtcs: {}   conectores: {}   planos: {}",
        recursos.crtcs().len(),
        recursos.connectors().len(),
        card.plane_handles().map(|p| p.len()).unwrap_or(0)
    );
    // Cero entradas es que no hay curva de gamma y la luz nocturna no puede
    // teñir esa pantalla.
    let gamma: Vec<String> = recursos
        .crtcs()
        .iter()
        .filter_map(|c| card.get_crtc(*c).ok())
        .map(|c| c.gamma_length().to_string())
        .collect();
    println!("  tabla de gamma por crtc: {}", gamma.join(", "));

    for handle in recursos.connectors() {
        let Ok(con) = card.get_connector(*handle, false) else {
            continue;
        };
        let estado = match con.state() {
            connector::State::Connected => "conectado",
            connector::State::Disconnected => "desconectado",
            connector::State::Unknown => "desconocido",
        };
        let nombre = format!("{:?}-{}", con.interface(), con.interface_id());
        println!("  · {nombre}: {estado}");

        if con.state() != connector::State::Connected {
            continue;
        }
        let (mm_w, mm_h) = con.size().unwrap_or((0, 0));
        println!("      tamaño físico: {mm_w}×{mm_h} mm");
        // El modo preferido es el primero que anuncia el conector.
        for (i, modo) in con.modes().iter().take(4).enumerate() {
            let (w, h) = modo.size();
            println!(
                "      modo{}: {}×{} @ {} Hz",
                if i == 0 { " preferido" } else { "        " },
                w,
                h,
                modo.vrefresh(),
            );
        }
        if con.modes().len() > 4 {
            println!("      … y {} modos más", con.modes().len() - 4);
        }
    }
    Ok(())
}
