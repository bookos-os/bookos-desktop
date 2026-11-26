//! Widget del panel para el cambio rápido de usuario (estilo macOS).
//!
//! Se oculta automáticamente (`ancho() == 0.0`) cuando solo existe una cuenta
//! humana listable en `/etc/passwd` (HIG §12). Al pulsar abre [`Emergente::Usuarios`].

use std::time::SystemTime;

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

const PASSWD: &str = "/etc/passwd";

/// Widget de cambio rápido de usuario en el panel superior.
pub struct Usuarios {
    icono: Option<Icono>,
    varias: bool,
    actual: String,
    mtime: Option<SystemTime>,
}

fn passwd_mtime() -> Option<SystemTime> {
    std::fs::metadata(PASSWD).ok()?.modified().ok()
}

fn leer_estado(passwd: &str, usuario_actual: &str) -> (bool, String) {
    let cuentas = crate::bloqueo::listables(passwd);
    let varias = cuentas.len() > 1;
    let actual = cuentas
        .into_iter()
        .find(|(cuenta, _, _)| cuenta == usuario_actual)
        .map(|(_, _, nombre)| nombre)
        .unwrap_or_else(|| usuario_actual.to_string());
    (varias, actual)
}

impl Usuarios {
    pub fn new() -> Self {
        let usuario = std::env::var("USER").unwrap_or_default();
        let passwd = std::fs::read_to_string(PASSWD).unwrap_or_default();
        let (varias, actual) = leer_estado(&passwd, &usuario);
        Self {
            icono: icono::propio("usuarios"),
            varias,
            actual,
            mtime: passwd_mtime(),
        }
    }

    #[cfg(test)]
    pub(crate) fn desde_passwd(passwd: &str, usuario_actual: &str) -> Self {
        let (varias, actual) = leer_estado(passwd, usuario_actual);
        Self {
            icono: icono::propio("usuarios"),
            varias,
            actual,
            mtime: None,
        }
    }
}

impl Widget for Usuarios {
    fn nombre(&self) -> &'static str {
        "usuarios"
    }

    fn accesible(&self) -> (&'static str, String) {
        (
            crate::emergente::t("Cambiar de usuario", "Switch user"),
            self.actual.clone(),
        )
    }

    fn refrescar(&mut self) -> bool {
        let nuevo_mtime = passwd_mtime();
        if nuevo_mtime == self.mtime {
            return false;
        }
        self.mtime = nuevo_mtime;
        let usuario = std::env::var("USER").unwrap_or_default();
        let passwd = std::fs::read_to_string(PASSWD).unwrap_or_default();
        let (varias, actual) = leer_estado(&passwd, &usuario);
        let cambio = self.varias != varias || self.actual != actual;
        self.varias = varias;
        self.actual = actual;
        cambio
    }

    fn ancho(&self) -> f32 {
        if self.varias && self.icono.is_some() {
            tema::ICONO_PANEL
        } else {
            0.0
        }
    }

    fn ver(&self) -> PanelElement<'_> {
        if !self.varias {
            return crate::widget::vacio();
        }
        match &self.icono {
            Some(ic) => icono::ver_teñido(ic, tema::ICONO_PANEL, Some(TEXT())),
            None => crate::widget::vacio(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_sola_cuenta_no_ocupa_sitio_en_el_panel() {
        let passwd = "eve:x:1000:1000:Evelyn,,,:/home/eve:/bin/bash\n\
                      nobody:x:65534:65534:Kernel Overflow User:/:/sbin/nologin\n";
        let w = Usuarios::desde_passwd(passwd, "eve");
        assert_eq!(w.ancho(), 0.0);
    }

    #[test]
    fn varias_cuentas_muestran_el_widget_y_su_estado_accesible() {
        let passwd = "eve:x:1000:1000:Evelyn,,,:/home/eve:/bin/bash\n\
                      alex:x:1001:1001:Alex,,,:/home/alex:/bin/bash\n";
        let w = Usuarios::desde_passwd(passwd, "eve");
        assert_eq!(w.ancho(), tema::ICONO_PANEL);
        let (_, valor) = w.accesible();
        assert_eq!(valor, "Evelyn");
    }
}
