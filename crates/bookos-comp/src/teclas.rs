//! Remapeo de teclas: `~/.config/bookos/teclas.conf`.
//!
//! Sirve para las teclas que en Linux no hacen nada —la de Copilot, la de algún
//! fabricante— y para cambiar una tecla por otra. Se aplica **antes** de que la
//! tecla entre en xkb, sustituyendo el código evdev: así xkb recalcula los
//! modificadores solo y los atajos del compositor ven la tecla ya traducida.
//!
//! Una línea por remapeo, `origen = destino`:
//!
//! ```text
//! copilot = ctrl
//! evdev:190 = accion:actividades
//! evdev:191 = cmd:konsole
//! menu = nada
//! ```
//!
//! **Copilot no es una tecla.** El firmware manda `Meta` izquierda, `Mayús`
//! izquierda y `F23`, en ese orden, y las suelta al revés. Cuando llega la F23
//! las dos primeras ya han salido hacia el cliente con foco: no hay forma de
//! saber que eran Copilot hasta entonces sin retener todas las pulsaciones de
//! Meta, que retrasaría cada atajo de la sesión. Se sueltan a mano en ese
//! momento y se tragan sus sueltas físicas. El cliente ve Meta+Mayús el rato que
//! separa las dos pulsaciones; no se ha medido en esta máquina.
//!
//! **Fn no aparece.** La resuelve el firmware del teclado y nunca llega al
//! sistema (ver la nota de Impr en `keybinds.rs`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const KEY_LEFTSHIFT: u32 = 42;
const KEY_F23: u32 = 193;
const KEY_LEFTMETA: u32 = 125;
/// `KEY_MAX` de `linux/input-event-codes.h`.
const KEY_MAX: u32 = 0x2ff;

/// Los nombres que ofrece la app, con su código de `input-event-codes.h`.
/// Cualquier otra tecla se escribe como `evdev:N`.
const NOMBRES: &[(&str, u32)] = &[
    ("ctrl", 29),
    ("ctrl_der", 97),
    ("shift", 42),
    ("shift_der", 54),
    ("alt", 56),
    ("altgr", 100),
    ("meta", 125),
    ("meta_der", 126),
    ("esc", 1),
    ("retroceso", 14),
    ("tab", 15),
    ("intro", 28),
    ("espacio", 57),
    ("bloqmayus", 58),
    ("impr", 99),
    ("inicio", 102),
    ("arriba", 103),
    ("repag", 104),
    ("izquierda", 105),
    ("derecha", 106),
    ("fin", 107),
    ("abajo", 108),
    ("avpag", 109),
    ("insert", 110),
    ("supr", 111),
    ("silencio", 113),
    ("bajar_volumen", 114),
    ("subir_volumen", 115),
    ("menu", 127),
    ("siguiente", 163),
    ("reproducir", 164),
    ("anterior", 165),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origen {
    /// El acorde Meta+Mayús+F23 que manda la tecla de Copilot.
    Copilot,
    Evdev(u32),
}

/// Funciones del escritorio a las que se puede mandar una tecla. Lista cerrada:
/// son las que tienen sentido sin argumentos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccionRemap {
    Actividades,
    Asistente,
    Buscador,
    Launchpad,
    VistaEscritorios,
    MostrarEscritorio,
    Captura,
    Bloquear,
    ApagarPantalla,
    Salvapantallas,
    Terminal,
}

const ACCIONES: &[(&str, AccionRemap)] = &[
    ("actividades", AccionRemap::Actividades),
    ("asistente", AccionRemap::Asistente),
    ("buscador", AccionRemap::Buscador),
    ("launchpad", AccionRemap::Launchpad),
    ("vista_escritorios", AccionRemap::VistaEscritorios),
    ("mostrar_escritorio", AccionRemap::MostrarEscritorio),
    ("captura", AccionRemap::Captura),
    ("bloquear", AccionRemap::Bloquear),
    ("apagar_pantalla", AccionRemap::ApagarPantalla),
    ("salvapantallas", AccionRemap::Salvapantallas),
    ("terminal", AccionRemap::Terminal),
];

impl AccionRemap {
    pub fn accion(self) -> crate::keybinds::Accion {
        use crate::keybinds::Accion;
        match self {
            AccionRemap::Actividades => Accion::Actividades,
            AccionRemap::Asistente => Accion::Asistente,
            AccionRemap::Buscador => Accion::Buscador,
            AccionRemap::Launchpad => Accion::Launchpad,
            AccionRemap::VistaEscritorios => Accion::VistaEscritorios,
            AccionRemap::MostrarEscritorio => Accion::MostrarEscritorio,
            AccionRemap::Captura => Accion::Captura,
            AccionRemap::Bloquear => Accion::DelShell(bookos_shell::Accion::Bloquear),
            AccionRemap::ApagarPantalla => Accion::ApagarPantalla,
            AccionRemap::Salvapantallas => Accion::Salvapantallas,
            AccionRemap::Terminal => Accion::Terminal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destino {
    Tecla(u32),
    Accion(AccionRemap),
    Comando(String),
    /// La tecla se traga: ni llega al cliente ni hace nada.
    Nada,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remapeos(Vec<(Origen, Destino)>);

/// Lo que sale de traducir un evento físico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evento {
    /// Una tecla, en evdev, que sigue su camino hacia xkb. `true` es pulsada.
    Tecla(u32, bool),
    Accion(AccionRemap),
    Comando(String),
}

fn tecla_de_texto(texto: &str) -> Result<u32, String> {
    if let Some(n) = texto.strip_prefix("evdev:") {
        let codigo: u32 = n
            .trim()
            .parse()
            .map_err(|_| format!("código evdev no válido: «{texto}»"))?;
        if codigo == 0 || codigo > KEY_MAX {
            return Err(format!("código evdev fuera de rango: {codigo}"));
        }
        return Ok(codigo);
    }
    NOMBRES
        .iter()
        .find(|(nombre, _)| *nombre == texto)
        .map(|(_, codigo)| *codigo)
        .ok_or_else(|| format!("tecla desconocida: «{texto}»"))
}

fn tecla_a_texto(codigo: u32) -> String {
    NOMBRES
        .iter()
        .find(|(_, c)| *c == codigo)
        .map(|(nombre, _)| (*nombre).to_string())
        .unwrap_or_else(|| format!("evdev:{codigo}"))
}

impl Origen {
    fn desde_texto(texto: &str) -> Result<Self, String> {
        match texto {
            "copilot" => Ok(Origen::Copilot),
            otro => tecla_de_texto(otro).map(Origen::Evdev),
        }
    }

    fn a_texto(self) -> String {
        match self {
            Origen::Copilot => "copilot".into(),
            Origen::Evdev(codigo) => tecla_a_texto(codigo),
        }
    }
}

impl Destino {
    fn desde_texto(texto: &str) -> Result<Self, String> {
        if texto == "nada" {
            return Ok(Destino::Nada);
        }
        if let Some(nombre) = texto.strip_prefix("accion:") {
            return ACCIONES
                .iter()
                .find(|(n, _)| *n == nombre)
                .map(|(_, a)| Destino::Accion(*a))
                .ok_or_else(|| format!("acción desconocida: «{nombre}»"));
        }
        if let Some(cmd) = texto.strip_prefix("cmd:") {
            let cmd = cmd.trim();
            if cmd.is_empty() {
                return Err("el comando está vacío".into());
            }
            return Ok(Destino::Comando(cmd.to_string()));
        }
        tecla_de_texto(texto).map(Destino::Tecla)
    }

    fn a_texto(&self) -> String {
        match self {
            Destino::Tecla(codigo) => tecla_a_texto(*codigo),
            Destino::Accion(accion) => {
                let nombre = ACCIONES
                    .iter()
                    .find(|(_, a)| a == accion)
                    .map(|(n, _)| *n)
                    .expect("toda AccionRemap está en ACCIONES");
                format!("accion:{nombre}")
            }
            Destino::Comando(cmd) => format!("cmd:{cmd}"),
            Destino::Nada => "nada".into(),
        }
    }
}

/// Una entrada tal como viaja por D-Bus: los mismos textos que en el fichero,
/// para que la app no tenga que conocer dos formatos.
#[derive(Serialize, Deserialize)]
struct EntradaJson {
    origen: String,
    destino: String,
}

impl Remapeos {
    pub fn buscar(&self, origen: Origen) -> Option<&Destino> {
        self.0.iter().find(|(o, _)| *o == origen).map(|(_, d)| d)
    }

    fn validar(entradas: Vec<(Origen, Destino)>) -> Result<Self, String> {
        for (i, (origen, _)) in entradas.iter().enumerate() {
            if entradas[..i].iter().any(|(o, _)| o == origen) {
                return Err(format!("la tecla «{}» está repetida", origen.a_texto()));
            }
        }
        Ok(Remapeos(entradas))
    }

    /// Lectura estricta: una línea mala invalida todo. Es la que usa D-Bus,
    /// donde el error vuelve a quien lo puede enseñar.
    fn desde_texto(texto: &str) -> Result<Self, String> {
        let entradas = texto
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|linea| {
                let (origen, destino) = linea
                    .split_once('=')
                    .ok_or_else(|| format!("línea sin «=»: «{linea}»"))?;
                Ok((
                    Origen::desde_texto(origen.trim())?,
                    Destino::desde_texto(destino.trim())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::validar(entradas)
    }

    fn a_texto(&self) -> String {
        let mut salida = String::from("# Remapeo de teclas de BookOS: origen = destino.\n");
        for (origen, destino) in &self.0 {
            salida.push_str(&format!("{} = {}\n", origen.a_texto(), destino.a_texto()));
        }
        salida
    }

    pub fn desde_json(json: &str) -> Result<Self, String> {
        let entradas: Vec<EntradaJson> =
            serde_json::from_str(json).map_err(|e| format!("JSON no válido: {e}"))?;
        let entradas = entradas
            .iter()
            .map(|e| {
                // Un salto de línea partiría la entrada en dos al escribirla.
                if e.destino.contains(['\n', '\r']) {
                    return Err("el destino no puede llevar saltos de línea".to_string());
                }
                Ok((
                    Origen::desde_texto(e.origen.trim())?,
                    Destino::desde_texto(e.destino.trim())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::validar(entradas)
    }

    pub fn a_json(&self) -> String {
        let entradas: Vec<EntradaJson> = self
            .0
            .iter()
            .map(|(o, d)| EntradaJson {
                origen: o.a_texto(),
                destino: d.a_texto(),
            })
            .collect();
        serde_json::to_string(&entradas).expect("un Vec de cadenas siempre serializa")
    }
}

fn ruta() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("bookos/teclas.conf"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/bookos/teclas.conf"))
}

/// Sin fichero no hay remapeos. Con un fichero roto tampoco, y se dice: aplicar
/// la mitad buena dejaría teclas cambiadas que el usuario no reconoce.
pub fn cargar() -> Remapeos {
    let Some(ruta) = ruta() else {
        return Remapeos::default();
    };
    let Ok(texto) = std::fs::read_to_string(&ruta) else {
        return Remapeos::default();
    };
    Remapeos::desde_texto(&texto).unwrap_or_else(|err| {
        tracing::warn!(ruta = %ruta.display(), "teclas.conf no válido, sin remapeos: {err}");
        Remapeos::default()
    })
}

pub fn guardar(remapeos: &Remapeos) -> std::io::Result<()> {
    let Some(ruta) = ruta() else {
        return Err(std::io::Error::other("sin HOME ni XDG_CONFIG_HOME"));
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(ruta, remapeos.a_texto())
}

/// Lo que hay que recordar entre eventos para que ninguna tecla se quede
/// trabada.
#[derive(Debug, Default)]
pub struct Traductor {
    /// Teclas físicas pulsadas. Solo importan Meta y Mayús, para reconocer el
    /// acorde de Copilot sin fiarse de los modificadores de xkb, que ya pueden
    /// venir remapeados.
    fisicas: Vec<u32>,
    /// Origen pulsado y el destino con el que se pulsó. La suelta usa este y no
    /// el mapa actual: si la configuración cambia con la tecla abajo, se suelta
    /// lo que se pulsó. `None` es que la pulsación no salió hacia xkb.
    pulsadas: Vec<(u32, Option<u32>)>,
    /// Sueltas físicas que ya no deben llegar a nadie: las de Meta y Mayús tras
    /// Copilot, que ya se soltaron a mano, y la de una tecla capturada.
    tragar: Vec<u32>,
}

impl Traductor {
    /// ¿Esta pulsación es la F23 del acorde de Copilot?
    pub fn es_copilot(&self, evdev: u32) -> bool {
        evdev == KEY_F23
            && self.fisicas.contains(&KEY_LEFTMETA)
            && self.fisicas.contains(&KEY_LEFTSHIFT)
    }

    /// La pulsación se la quedó otro —la captura de la app—: que su suelta
    /// tampoco salga.
    pub fn tragar_suelta(&mut self, evdev: u32) {
        self.tragar.push(evdev);
    }

    pub fn traducir(&mut self, mapa: &Remapeos, evdev: u32, pulsada: bool) -> Vec<Evento> {
        if pulsada {
            self.fisicas.push(evdev);
        } else {
            self.fisicas.retain(|&c| c != evdev);
            if let Some(i) = self.tragar.iter().position(|&c| c == evdev) {
                self.tragar.swap_remove(i);
                return Vec::new();
            }
            if let Some(i) = self.pulsadas.iter().position(|&(o, _)| o == evdev) {
                let (_, destino) = self.pulsadas.swap_remove(i);
                return destino
                    .map(|d| Evento::Tecla(d, false))
                    .into_iter()
                    .collect();
            }
            return vec![Evento::Tecla(evdev, false)];
        }

        let copilot = self.es_copilot(evdev);
        let origen = if copilot {
            Origen::Copilot
        } else {
            Origen::Evdev(evdev)
        };
        let Some(destino) = mapa.buscar(origen) else {
            return vec![Evento::Tecla(evdev, true)];
        };

        let mut eventos = Vec::new();
        if copilot {
            eventos.push(Evento::Tecla(KEY_LEFTMETA, false));
            eventos.push(Evento::Tecla(KEY_LEFTSHIFT, false));
            self.tragar.extend([KEY_LEFTMETA, KEY_LEFTSHIFT]);
        }
        let salida = match destino {
            Destino::Tecla(d) => {
                eventos.push(Evento::Tecla(*d, true));
                Some(*d)
            }
            Destino::Accion(a) => {
                eventos.push(Evento::Accion(*a));
                None
            }
            Destino::Comando(cmd) => {
                eventos.push(Evento::Comando(cmd.clone()));
                None
            }
            Destino::Nada => None,
        };
        self.pulsadas.push((evdev, salida));
        eventos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapa(texto: &str) -> Remapeos {
        Remapeos::desde_texto(texto).expect("mapa de prueba válido")
    }

    #[test]
    fn el_fichero_va_y_vuelve_igual() {
        let original = mapa(
            "# comentario\ncopilot = ctrl\nevdev:190 = accion:actividades\n\
             menu = cmd:konsole --new-tab\nsupr = nada\n",
        );
        assert_eq!(
            Remapeos::desde_texto(&original.a_texto()),
            Ok(original.clone())
        );
        assert_eq!(Remapeos::desde_json(&original.a_json()), Ok(original));
    }

    #[test]
    fn un_origen_repetido_se_rechaza() {
        let err = Remapeos::desde_texto("copilot = ctrl\ncopilot = alt\n").unwrap_err();
        assert!(err.contains("repetida"), "{err}");
    }

    #[test]
    fn se_rechazan_nombres_y_codigos_invalidos() {
        assert!(Remapeos::desde_texto("fn = ctrl").is_err());
        assert!(Remapeos::desde_texto("evdev:0 = ctrl").is_err());
        assert!(Remapeos::desde_texto("evdev:768 = ctrl").is_err());
        assert!(Remapeos::desde_texto("copilot = cmd:   ").is_err());
        assert!(Remapeos::desde_texto("copilot = accion:volar").is_err());
        assert!(Remapeos::desde_json(r#"[{"origen":"copilot","destino":"cmd:a\nb"}]"#).is_err());
    }

    #[test]
    fn sin_remapeo_las_teclas_pasan_tal_cual() {
        let mut t = Traductor::default();
        let vacio = Remapeos::default();
        assert_eq!(t.traducir(&vacio, 30, true), vec![Evento::Tecla(30, true)]);
        assert_eq!(
            t.traducir(&vacio, 30, false),
            vec![Evento::Tecla(30, false)]
        );
    }

    #[test]
    fn copilot_a_ctrl_suelta_meta_y_mayus_una_sola_vez() {
        let m = mapa("copilot = ctrl");
        let mut t = Traductor::default();
        // La secuencia que manda el firmware: 125↓ 42↓ 193↓ 193↑ 42↑ 125↑.
        let mut salida = Vec::new();
        for (codigo, pulsada) in [
            (125, true),
            (42, true),
            (193, true),
            (193, false),
            (42, false),
            (125, false),
        ] {
            salida.extend(t.traducir(&m, codigo, pulsada));
        }
        assert_eq!(
            salida,
            vec![
                Evento::Tecla(125, true),
                Evento::Tecla(42, true),
                Evento::Tecla(125, false),
                Evento::Tecla(42, false),
                Evento::Tecla(29, true),
                Evento::Tecla(29, false),
            ]
        );
    }

    #[test]
    fn f23_sin_el_acorde_no_es_copilot() {
        let m = mapa("copilot = ctrl");
        let mut t = Traductor::default();
        assert_eq!(t.traducir(&m, 193, true), vec![Evento::Tecla(193, true)]);
    }

    #[test]
    fn copilot_a_accion_la_lanza_y_no_deja_nada_pulsado() {
        let m = mapa("copilot = accion:actividades");
        let mut t = Traductor::default();
        t.traducir(&m, 125, true);
        t.traducir(&m, 42, true);
        assert_eq!(
            t.traducir(&m, 193, true),
            vec![
                Evento::Tecla(125, false),
                Evento::Tecla(42, false),
                Evento::Accion(AccionRemap::Actividades),
            ]
        );
        assert!(t.traducir(&m, 193, false).is_empty());
        assert!(t.traducir(&m, 42, false).is_empty());
        assert!(t.traducir(&m, 125, false).is_empty());
    }

    #[test]
    fn la_tecla_copilot_puede_activar_el_asistente() {
        let m = mapa("copilot = accion:asistente");
        let mut t = Traductor::default();
        t.traducir(&m, 125, true);
        t.traducir(&m, 42, true);
        assert_eq!(
            t.traducir(&m, 193, true).last(),
            Some(&Evento::Accion(AccionRemap::Asistente))
        );
    }

    #[test]
    fn se_suelta_el_destino_con_el_que_se_pulso() {
        let mut t = Traductor::default();
        assert_eq!(
            t.traducir(&mapa("menu = ctrl"), 127, true),
            vec![Evento::Tecla(29, true)]
        );
        // Entre medias se recarga con otro destino.
        assert_eq!(
            t.traducir(&mapa("menu = alt"), 127, false),
            vec![Evento::Tecla(29, false)]
        );
    }

    #[test]
    fn la_suelta_de_una_tecla_capturada_no_sale() {
        let mut t = Traductor::default();
        let vacio = Remapeos::default();
        t.tragar_suelta(190);
        assert!(t.traducir(&vacio, 190, false).is_empty());
    }
}
