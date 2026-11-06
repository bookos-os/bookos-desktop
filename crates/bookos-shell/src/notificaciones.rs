//! Las notificaciones que llegan de las aplicaciones, guardadas aquí.
//!
//! El shell no habla D-Bus —no puede: tiene que estar pintado en el primer
//! frame, antes de que haya bus con quien hablar—, así que quien atiende
//! `org.freedesktop.Notifications` es el compositor, en un hilo aparte, y le
//! entrega aquí lo que llega ya masticado. Este módulo es solo la cola: qué hay
//! sin leer, en qué orden y con qué icono.
//!
//! **El icono se resuelve al llegar, no al dibujar.** Buscar un icono en el
//! tema son unos cientos de `stat` (ver [`crate::icono`]), y hacerlo dentro de
//! `view` significaría pagarlos en cada repintado de la tarjeta.

use std::time::Instant;

use crate::icono::{self, Icono};

/// Cuántas se guardan. Las más viejas se caen por abajo.
///
/// Veinte y no «todas»: la tarjeta enseña las últimas y el resto no lo mira
/// nadie, pero cada una sostiene su icono cargado y un `String` por campo.
pub const MAXIMO: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accion {
    pub clave: String,
    pub etiqueta: String,
}

/// Una notificación tal y como la enseña el shell.
#[derive(Clone)]
pub struct Notificacion {
    /// El identificador de D-Bus, con el que la aplicación puede cerrarla o
    /// sustituirla.
    pub id: u32,
    /// Quién avisa: el `app_name` de la llamada.
    pub app: String,
    pub resumen: String,
    pub cuerpo: String,
    /// Ya cargado. `None` si la aplicación no dio ninguno o no se encontró.
    pub icono: Option<Icono>,
    /// Urgencia crítica (2 en la especificación). Las críticas no se silencian
    /// con «No molestar»: para eso están.
    pub critica: bool,
    pub acciones: Vec<Accion>,
    pub progreso: Option<u8>,
    pub progreso_indeterminado: bool,
    /// Se marca al abrir el historial; las nuevas siguen destacadas en el panel.
    pub leida: bool,
    /// Cuándo llegó, para el «hace 3 min» de la tarjeta.
    pub llegada: Instant,
}

impl Notificacion {
    /// Construye la notificación resolviendo su icono.
    ///
    /// `icono` es lo que manda la aplicación en `app_icon`: un nombre del tema
    /// («firefox»), una ruta a un fichero, o nada. Cuando no dice nada se prueba
    /// con el nombre de la aplicación, que acierta a menudo —Firefox se
    /// presenta como «Firefox» y su icono se llama `firefox`— y es mejor que
    /// dejar el hueco vacío.
    // 0.1.0
    pub fn nueva(
        id: u32,
        app: String,
        resumen: String,
        cuerpo: String,
        icono: &str,
        critica: bool,
    ) -> Self {
        Self::nueva_con_datos(
            id,
            app,
            resumen,
            cuerpo,
            icono,
            critica,
            Vec::new(),
            None,
            false,
        )
    }

    // Son los campos de Notify tal cual llegan por D-Bus; agruparlos en una
    // estructura solo movería la misma lista a otro sitio.
    #[allow(clippy::too_many_arguments)]
    // 0.1.0
    pub fn nueva_con_datos(
        id: u32,
        app: String,
        resumen: String,
        cuerpo: String,
        icono: &str,
        critica: bool,
        acciones: Vec<Accion>,
        progreso: Option<u8>,
        progreso_indeterminado: bool,
    ) -> Self {
        let icono = icono::cargar(icono)
            .or_else(|| icono::cargar(&app.to_lowercase()))
            .or_else(|| icono::propio("notificaciones"));
        Self {
            id,
            app,
            resumen,
            cuerpo,
            icono,
            critica,
            acciones,
            progreso,
            progreso_indeterminado,
            leida: false,
            llegada: Instant::now(),
        }
    }

    /// «ahora», «hace 3 min», «hace 2 h». Lo que se lee en la esquina de la
    /// fila; los segundos exactos no le importan a nadie.
    // 0.1.0
    pub fn hace(&self) -> String {
        let s = self.llegada.elapsed().as_secs();
        match s {
            0..=59 => "ahora".into(),
            60..=3599 => format!("hace {} min", s / 60),
            _ => format!("hace {} h", s / 3600),
        }
    }
}

/// La cola de notificaciones del escritorio.
#[derive(Default)]
pub struct Registro {
    /// De la más nueva a la más vieja: es el orden en que se enseñan y evita
    /// invertir la lista en cada repintado.
    lista: Vec<Notificacion>,
}

impl Registro {
    /// Guarda una notificación. Si ya había una con ese `id` la **sustituye en
    /// su sitio**, que es lo que pide la especificación para `replaces_id`: un
    /// reproductor que avisa de cada canción tiene que actualizar su aviso, no
    /// llenar la lista con veinte.
    // 0.1.0
    pub fn añadir(&mut self, nueva: Notificacion) {
        if let Some(hueco) = self.lista.iter_mut().find(|n| n.id == nueva.id) {
            *hueco = nueva;
            return;
        }
        self.lista.insert(0, nueva);
        self.lista.truncate(MAXIMO);
    }

    /// Quita la del `id`. `true` si estaba.
    // 0.1.0
    pub fn cerrar(&mut self, id: u32) -> bool {
        let antes = self.lista.len();
        self.lista.retain(|n| n.id != id);
        self.lista.len() != antes
    }

    /// Vacía la cola. Devuelve los identificadores que había, porque a cada uno
    /// hay que avisarle por D-Bus de que su notificación se cerró.
    // 0.1.0
    pub fn vaciar(&mut self) -> Vec<u32> {
        self.lista.drain(..).map(|n| n.id).collect()
    }

    // 0.1.0
    pub fn lista(&self) -> &[Notificacion] {
        &self.lista
    }

    // 0.1.0
    pub fn cuantas(&self) -> u32 {
        self.lista.iter().filter(|n| !n.leida).count() as u32
    }

    /// Marca el historial como leído y devuelve si cambió algo.
    // 0.1.0
    pub fn marcar_leidas(&mut self) -> bool {
        let mut cambio = false;
        for n in &mut self.lista {
            if !n.leida {
                n.leida = true;
                cambio = true;
            }
        }
        cambio
    }

    // 0.1.0
    pub fn hay(&self) -> bool {
        !self.lista.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, resumen: &str) -> Notificacion {
        Notificacion {
            id,
            app: "prueba".into(),
            resumen: resumen.into(),
            cuerpo: String::new(),
            icono: None,
            critica: false,
            acciones: Vec::new(),
            progreso: None,
            progreso_indeterminado: false,
            leida: false,
            llegada: Instant::now(),
        }
    }

    /// La más nueva va arriba y el mismo `id` sustituye en su sitio: un
    /// reproductor que avisa de cada canción actualiza su aviso en vez de
    /// llenar la lista.
    #[test]
    fn el_mismo_id_sustituye_sin_moverse() {
        let mut r = Registro::default();
        r.añadir(n(1, "una"));
        r.añadir(n(2, "dos"));
        assert_eq!(r.lista()[0].resumen, "dos", "la última va arriba");
        r.añadir(n(1, "una corregida"));
        assert_eq!(r.cuantas(), 2, "sustituir no añade");
        assert_eq!(
            r.lista()[1].resumen,
            "una corregida",
            "y no se mueve de sitio"
        );
    }

    #[test]
    fn la_cola_tiene_tope_y_se_vacia() {
        let mut r = Registro::default();
        for i in 0..(MAXIMO as u32 + 5) {
            r.añadir(n(i, "x"));
        }
        assert_eq!(r.cuantas(), MAXIMO as u32);
        // Las que se caen son las viejas: arriba queda la última que entró.
        assert_eq!(r.lista()[0].id, MAXIMO as u32 + 4);

        assert!(r.cerrar(MAXIMO as u32 + 4));
        assert!(!r.cerrar(9999), "cerrar una que no está no dice que sí");

        let ids = r.vaciar();
        assert_eq!(ids.len(), MAXIMO - 1, "hay que avisar a cada una");
        assert!(!r.hay());
    }

    #[test]
    fn el_tiempo_se_lee_en_palabras() {
        let mut reciente = n(1, "x");
        assert_eq!(reciente.hace(), "ahora");
        reciente.llegada = Instant::now() - std::time::Duration::from_secs(200);
        assert_eq!(reciente.hace(), "hace 3 min");
        reciente.llegada = Instant::now() - std::time::Duration::from_secs(7300);
        assert_eq!(reciente.hace(), "hace 2 h");
    }

    #[test]
    fn solo_las_nuevas_cuentan_como_pendientes() {
        let mut r = Registro::default();
        r.añadir(n(1, "x"));
        r.añadir(n(2, "y"));
        assert_eq!(r.cuantas(), 2);
        assert!(r.marcar_leidas());
        assert_eq!(r.cuantas(), 0);
        assert!(!r.marcar_leidas());
    }
}
