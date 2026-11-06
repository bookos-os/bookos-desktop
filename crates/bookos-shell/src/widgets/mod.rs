//! Los widgets que trae BookOS de serie.
//!
//! Cada uno lee **solo lo suyo**. Antes un evento de udev de cualquier tipo
//! provocaba releer las cuatro fuentes de sysfs; ahora un cambio de brillo no
//! hace mirar el estado de la red.

mod bateria;
pub(crate) use bateria::icono_aviso as icono_aviso_bateria;
pub use bateria::sobre_fondo as bateria_sobre_fondo;
mod bateria_simple;
mod bluetooth;
mod brillo;
mod control;
pub(crate) mod escritorios;
mod medidor;
mod notificaciones;
mod red;
mod reloj;
pub(crate) mod volumen;

use crate::widget::Widget;

/// Todos los nombres que acepta [`por_nombre`], en el orden en que Ajustes los
/// ofrece. Es la lista que se enseña y la que valida lo que se guarda: un
/// nombre fuera de ella no construye nada, y quien lo escribe no se entera.
// añadido en la 0.2.0
pub const NOMBRES: &[&str] = &[
    "escritorios",
    "bateria",
    "bateria-simple",
    "bluetooth",
    "red",
    "volumen",
    "brillo",
    "notificaciones",
    "control",
    "reloj",
    "monitor",
    "trafico",
    "consumo",
];

/// Construye un widget por su nombre en la configuración.
///
/// Devuelve `None` para un nombre que no existe: una línea con una errata en
/// `panel.conf` deja fuera ese widget, no tumba el panel.
// 0.1.0
pub fn por_nombre(nombre: &str) -> Option<Box<dyn Widget>> {
    match nombre {
        "reloj" => Some(Box::new(reloj::Reloj::new())),
        "bateria" => Some(Box::new(bateria::Bateria::new())),
        // La misma batería con el trazo de los demás widgets, para quien
        // prefiera una fila monocroma. Ver `widgets/bateria_simple.rs`.
        "bateria-simple" => Some(Box::new(bateria_simple::BateriaSimple::new())),
        "red" => Some(Box::new(red::Red::new())),
        "brillo" => Some(Box::new(brillo::Brillo::new())),
        "volumen" => Some(Box::new(volumen::Volumen::new())),
        "bluetooth" => Some(Box::new(bluetooth::Bluetooth::new())),
        "notificaciones" => Some(Box::new(notificaciones::Notificaciones::new())),
        "control" => Some(Box::new(control::Control::new())),
        "escritorios" => Some(Box::new(escritorios::Escritorios::new())),
        "monitor" => Some(Box::new(medidor::Medidor::monitor())),
        "trafico" => Some(Box::new(medidor::Medidor::trafico())),
        "consumo" => Some(Box::new(medidor::Medidor::consumo())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El catálogo es lo que Ajustes ofrece y valida: un nombre en él que
    /// `por_nombre` no construye sería una opción que no hace nada.
    #[test]
    fn todo_lo_del_catalogo_se_construye_con_su_nombre() {
        for nombre in NOMBRES {
            let widget = por_nombre(nombre).unwrap_or_else(|| panic!("«{nombre}» no se construye"));
            // La batería de trazo simple se llama como la otra para abrir la
            // misma tarjeta al pulsarla; es lo único que no coincide.
            let esperado = if *nombre == "bateria-simple" {
                "bateria"
            } else {
                nombre
            };
            assert_eq!(widget.nombre(), esperado);
        }
    }

    #[test]
    fn el_catalogo_no_repite_nombres() {
        let mut vistos = std::collections::HashSet::new();
        assert!(NOMBRES.iter().all(|n| vistos.insert(*n)));
    }
}
