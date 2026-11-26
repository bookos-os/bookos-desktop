//! Tarjeta emergente de cambio rápido de usuario, anclada al widget `usuarios`.
//!
//! Sigue el patrón de las listas del sistema (`control::tarjeta`, `lista::cabecera`,
//! `lista::grupo`, `lista::ventana_alto`), con avatares circulares (foto o iniciales),
//! indicador de sesión abierta, navegación por teclado y acceso para Orca.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::lista;
use super::{Ancla, FilaAccesible, Tecla, t};

const BOTON_PIE: f32 = 44.0;
const BOTONES_PIE: usize = 2;
const PIE_TOTAL: f32 = lista::PIE_AIRE + BOTONES_PIE as f32 * BOTON_PIE + lista::HUECO_FILA;
const MAX_FILAS: usize = 5;
const MAX_VENTANA: f32 =
    MAX_FILAS as f32 * lista::FILA + (MAX_FILAS - 1) as f32 * lista::HUECO_FILA;
const AVATAR: f32 = lista::DISCO;
const AVATAR_PX: u32 = 72;
const PUNTO_SESION: f32 = 6.0;
const MAX_AVATAR_BYTES: u64 = 32 * 1024 * 1024;
const ANCHO_NOMBRE: f32 = lista::BALDOSA - 12.0 - AVATAR - 12.0 - 28.0;

#[derive(Clone)]
struct Cuenta {
    cuenta: String,
    nombre: String,
    actual: bool,
    abierta: bool,
    foto: Option<iced_widget::image::Handle>,
    iniciales: String,
}

/// Tarjeta emergente del widget de cambio rápido de usuario.
pub struct Usuarios {
    cuentas: Vec<Cuenta>,
    señalada: tema::Realce,
    pie: tema::Realce,
    desplazamiento: f32,
    foco: Option<Foco>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Foco {
    Cuenta(usize),
    Pie(usize),
}

fn iniciales_de(nombre: &str, cuenta: &str) -> String {
    let ini: String = nombre
        .split_whitespace()
        .filter_map(|p| p.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if ini.is_empty() {
        cuenta.chars().take(2).collect::<String>().to_uppercase()
    } else {
        ini
    }
}

fn cuenta_valida(cuenta: &str) -> bool {
    !cuenta.is_empty()
        && !cuenta.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, '\'' | '"' | '\\' | '`' | '$')
        })
}

fn avatar_publicado(cuenta: &str, uid: u32) -> Option<PathBuf> {
    if !cuenta_valida(cuenta) || cuenta.contains('/') || cuenta == "." || cuenta == ".." {
        return None;
    }
    let carpeta = Path::new(crate::APARIENCIA_PUBLICA).join(cuenta);
    let meta_carpeta = std::fs::symlink_metadata(&carpeta).ok()?;
    if !meta_carpeta.is_dir() || meta_carpeta.uid() != uid {
        return None;
    }
    let ruta = carpeta.join("avatar");
    let meta = std::fs::symlink_metadata(&ruta).ok()?;
    (meta.is_file() && meta.uid() == uid && meta.len() <= MAX_AVATAR_BYTES).then_some(ruta)
}

fn leer_avatar(
    cuenta: &str,
    uid: u32,
    actual: bool,
    avatar_conf: Option<&str>,
) -> Option<iced_widget::image::Handle> {
    let mut candidatas: Vec<PathBuf> = Vec::new();
    if actual {
        let casa = std::env::var("HOME").unwrap_or_default();
        if let Some(conf) = avatar_conf {
            candidatas.push(PathBuf::from(conf.replacen('~', &casa, 1)));
        }
        if !casa.is_empty() {
            candidatas.push(PathBuf::from(format!("{casa}/.face")));
            candidatas.push(PathBuf::from(format!("{casa}/.face.icon")));
        }
    }
    if let Some(pub_ruta) = avatar_publicado(cuenta, uid) {
        candidatas.push(pub_ruta);
    }
    if cuenta_valida(cuenta) && !cuenta.contains('/') {
        candidatas.push(PathBuf::from(format!(
            "/var/lib/AccountsService/icons/{cuenta}"
        )));
    }
    candidatas
        .into_iter()
        .find(|r| r.is_file())
        .and_then(|r| crate::bloqueo::foto_circular_de(r.to_str()?, AVATAR_PX))
}

fn ordenar_cuentas(cuentas: &mut [Cuenta]) {
    cuentas.sort_by(|a, b| {
        (!a.actual)
            .cmp(&!b.actual)
            .then_with(|| (!a.abierta).cmp(&!b.abierta))
            .then_with(|| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()))
            .then_with(|| a.cuenta.cmp(&b.cuenta))
    });
}

fn tinta_avatar_activo(fondo: Color) -> Color {
    let blanco = tema::contraste(Color::WHITE, fondo);
    let negro = tema::contraste(Color::BLACK, fondo);
    if blanco >= 4.5 {
        Color::WHITE
    } else if negro >= 4.5 {
        Color::BLACK
    } else if blanco >= negro {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

fn color_comprobado(fondo: Color) -> Color {
    let acento = tema::acento();
    if tema::contraste(acento, fondo) >= 3.0 {
        acento
    } else {
        tema::texto()
    }
}

fn color_punto_sesion(fondo: Color) -> Color {
    let base = tema::verde();
    if tema::contraste(base, fondo) >= 3.0 {
        return base;
    }
    (0..=20)
        .map(|n| tema::mezclar(base, tema::texto(), n as f32 * 0.05))
        .find(|c| tema::contraste(*c, fondo) >= 3.0)
        .unwrap_or_else(tema::texto)
}

impl Usuarios {
    pub fn new() -> Self {
        let propia = std::env::var("USER").unwrap_or_default();
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let avatar_conf = crate::Config::cargar().avatar;
        let mut cuentas: Vec<Cuenta> = crate::bloqueo::listables(&passwd)
            .into_iter()
            .map(|(cuenta, uid, nombre)| {
                let actual = cuenta == propia;
                let abierta = actual || Path::new(&format!("/run/user/{uid}")).exists();
                let foto = leer_avatar(&cuenta, uid, actual, avatar_conf.as_deref());
                let iniciales = iniciales_de(&nombre, &cuenta);
                Cuenta {
                    cuenta,
                    nombre,
                    actual,
                    abierta,
                    foto,
                    iniciales,
                }
            })
            .collect();
        ordenar_cuentas(&mut cuentas);
        Self {
            cuentas,
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            desplazamiento: 0.0,
            foco: None,
        }
    }

    pub(crate) fn desde_passwd(passwd: &str, propia: &str, abiertas: &[u32]) -> Self {
        let mut cuentas: Vec<Cuenta> = crate::bloqueo::listables(passwd)
            .into_iter()
            .map(|(cuenta, uid, nombre)| {
                let actual = cuenta == propia;
                let abierta = actual || abiertas.contains(&uid);
                let iniciales = iniciales_de(&nombre, &cuenta);
                Cuenta {
                    cuenta,
                    nombre,
                    actual,
                    abierta,
                    foto: None,
                    iniciales,
                }
            })
            .collect();
        ordenar_cuentas(&mut cuentas);
        Self {
            cuentas,
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            desplazamiento: 0.0,
            foco: None,
        }
    }

    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::BAJO_CABECERA + control::GRUPO
    }

    fn total(&self) -> f32 {
        let n = self.cuentas.len();
        if n == 0 {
            0.0
        } else {
            n as f32 * lista::FILA + (n - 1) as f32 * lista::HUECO_FILA
        }
    }

    fn alto_lista(&self) -> f32 {
        lista::ventana_de(self.total(), MAX_VENTANA)
    }

    fn maximo(&self) -> f32 {
        (self.total() - self.alto_lista()).max(0.0)
    }

    fn y_pie(&self) -> f32 {
        self.y_lista() + self.alto_lista() + lista::PIE_AIRE
    }

    pub fn size(&self) -> (f32, f32) {
        (
            lista::ANCHO,
            self.y_lista() + self.alto_lista() + PIE_TOTAL + control::GRUPO + lista::MARGEN,
        )
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("usuarios")
    }

    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.pie.animando()
    }

    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = lista::MARGEN + control::GRUPO;
        let y0 = self.y_lista();
        if x < x0 || x > x0 + lista::BALDOSA || y < y0 || y >= y0 + self.alto_lista() {
            return None;
        }
        let y_rel = y - y0 + self.desplazamiento;
        let paso = lista::FILA + lista::HUECO_FILA;
        let idx = (y_rel / paso) as usize;
        let c = self.cuentas.get(idx)?;
        if y_rel - idx as f32 * paso > lista::FILA || c.actual {
            return None;
        }
        Some(idx)
    }

    fn pie_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = lista::MARGEN + control::GRUPO;
        if x < x0 || x > x0 + lista::BALDOSA {
            return None;
        }
        let y0 = self.y_pie();
        for i in 0..BOTONES_PIE {
            let top = y0 + i as f32 * (BOTON_PIE + lista::HUECO_FILA);
            if (top..=top + BOTON_PIE).contains(&y) {
                return Some(i);
            }
        }
        None
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let fila = punto.and_then(|(x, y)| self.fila_en(x, y));
        let pie = punto.and_then(|(x, y)| self.pie_en(x, y));
        self.foco = fila.map(Foco::Cuenta).or_else(|| pie.map(Foco::Pie));
        let a = self.señalada.señalar(fila);
        let b = self.pie.señalar(pie);
        a || b
    }

    pub fn desplazar(&mut self, dy: f32) -> bool {
        let nuevo = (self.desplazamiento + dy).clamp(0.0, self.maximo());
        let cambio = (nuevo - self.desplazamiento).abs() > f32::EPSILON;
        self.desplazamiento = nuevo;
        cambio
    }

    fn mostrar(&mut self, idx: usize) {
        let y = if idx == 1 && self.cuentas.first().is_some_and(|c| c.actual) {
            0.0
        } else {
            idx as f32 * (lista::FILA + lista::HUECO_FILA)
        };
        let ventana = self.alto_lista();
        if y < self.desplazamiento {
            self.desplazamiento = y;
        } else if y + lista::FILA > self.desplazamiento + ventana {
            self.desplazamiento = y + lista::FILA - ventana;
        }
        self.desplazamiento = self.desplazamiento.clamp(0.0, self.maximo());
    }

    fn activar_cuenta(&self, idx: usize) -> Option<Accion> {
        let c = self.cuentas.get(idx)?;
        if c.actual || !cuenta_valida(&c.cuenta) {
            return None;
        }
        Some(Accion::Lanzar(format!(
            "bookos-cambiar-usuario '{}'",
            c.cuenta
        )))
    }

    fn activar_pie(&self, idx: usize) -> Option<Accion> {
        match idx {
            0 => Some(Accion::Lanzar("bookos-cambiar-usuario".into())),
            1 => Some(Accion::Lanzar("bookos-settings --page cuentas".into())),
            _ => None,
        }
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.pie_en(x, y) {
            return self.activar_pie(i);
        }
        let i = self.fila_en(x, y)?;
        self.activar_cuenta(i)
    }

    fn focos(&self) -> Vec<Foco> {
        let mut v: Vec<Foco> = self
            .cuentas
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.actual)
            .map(|(i, _)| Foco::Cuenta(i))
            .collect();
        for i in 0..BOTONES_PIE {
            v.push(Foco::Pie(i));
        }
        v
    }

    fn aplicar_foco(&mut self, foco: Option<Foco>) {
        self.foco = foco;
        match foco {
            Some(Foco::Cuenta(i)) => {
                self.señalada.señalar(Some(i));
                self.pie.señalar(None);
                self.mostrar(i);
            }
            Some(Foco::Pie(i)) => {
                self.señalada.señalar(None);
                self.pie.señalar(Some(i));
            }
            None => {
                self.señalada.señalar(None);
                self.pie.señalar(None);
            }
        }
    }

    fn mover(&mut self, paso: isize) {
        let focos = self.focos();
        if focos.is_empty() {
            return;
        }
        let pos = self.foco.and_then(|f| focos.iter().position(|x| *x == f));
        let next = match pos {
            None if paso > 0 => 0,
            None => focos.len() - 1,
            Some(i) => (i as isize + paso).clamp(0, focos.len() as isize - 1) as usize,
        };
        self.aplicar_foco(Some(focos[next]));
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Arriba => {
                self.mover(-1);
                Tecla::Consumida
            }
            T::Abajo | T::Tabulador => {
                self.mover(1);
                Tecla::Consumida
            }
            T::Inicio => {
                let primero = self.focos().first().copied();
                self.aplicar_foco(primero);
                Tecla::Consumida
            }
            T::Fin => {
                let ultimo = self.focos().last().copied();
                self.aplicar_foco(ultimo);
                Tecla::Consumida
            }
            T::Intro | T::Caracter(' ') => match self.foco {
                Some(Foco::Cuenta(i)) => self
                    .activar_cuenta(i)
                    .map_or(Tecla::Consumida, Tecla::Hacer),
                Some(Foco::Pie(i)) => self.activar_pie(i).map_or(Tecla::Consumida, Tecla::Hacer),
                None => Tecla::Ignorada,
            },
            _ => Tecla::Ignorada,
        }
    }

    pub fn accesible(&self) -> Vec<FilaAccesible> {
        use accesskit::Role;
        let mut items = Vec::new();
        let x0 = lista::MARGEN + control::GRUPO;
        let y0 = self.y_lista();
        let alto = self.alto_lista();
        let paso = lista::FILA + lista::HUECO_FILA;

        for (i, c) in self.cuentas.iter().enumerate() {
            let y = y0 + i as f32 * paso - self.desplazamiento;
            if y < y0 - 0.5 || y + lista::FILA > y0 + alto + 0.5 {
                continue;
            }
            let valor = if c.actual {
                t("Sesión actual", "Current session").to_string()
            } else if c.abierta {
                t("Sesión abierta", "Logged in").to_string()
            } else {
                t("Cambiar de cuenta…", "Switch account…").to_string()
            };
            items.push(FilaAccesible {
                id: format!("usuario:{}", c.cuenta),
                rol: Role::ListBoxOption,
                nombre: c.nombre.clone(),
                valor,
                rect: (x0, y, lista::BALDOSA, lista::FILA),
                marcada: c.actual.then_some(true),
                seleccionada: self.foco == Some(Foco::Cuenta(i)),
            });
        }

        let y_pie = self.y_pie();
        items.push(FilaAccesible {
            id: "usuarios:login".into(),
            rol: Role::Button,
            nombre: t("Ventana de inicio de sesión…", "Login Window…").into(),
            valor: String::new(),
            rect: (x0, y_pie, lista::BALDOSA, BOTON_PIE),
            marcada: None,
            seleccionada: self.foco == Some(Foco::Pie(0)),
        });
        items.push(FilaAccesible {
            id: "usuarios:ajustes".into(),
            rol: Role::Button,
            nombre: t("Ajustes de cuentas…", "Account Settings…").into(),
            valor: String::new(),
            rect: (
                x0,
                y_pie + BOTON_PIE + lista::HUECO_FILA,
                lista::BALDOSA,
                BOTON_PIE,
            ),
            marcada: None,
            seleccionada: self.foco == Some(Foco::Pie(1)),
        });
        items
    }

    pub fn view(&self) -> PanelElement<'_> {
        let cabecera = lista::cabecera(t("Cambiar de usuario", "Fast User Switching"), None, false);

        let cuerpo: PanelElement<'_> = if self.cuentas.is_empty() {
            lista::vacia(t("No hay cuentas", "No accounts"))
        } else {
            let mut filas = column![];
            for (i, c) in self.cuentas.iter().enumerate() {
                if i > 0 {
                    filas = filas.push(Space::new().height(Length::Fixed(lista::HUECO_FILA)));
                }
                filas = filas.push(self.fila_cuenta(c, self.señalada.intensidad(i)));
            }
            lista::ventana_alto(
                filas.into(),
                self.alto_lista(),
                self.maximo(),
                self.desplazamiento,
            )
        };

        let pie = column![
            Space::new().height(Length::Fixed(lista::HUECO_FILA + 2.0)),
            control::divisor(lista::BALDOSA),
            Space::new().height(Length::Fixed(2.0 + lista::HUECO_FILA)),
            self.boton_pie(
                t("Ventana de inicio de sesión…", "Login Window…"),
                Some("usuarios"),
                self.pie.intensidad(0),
            ),
            Space::new().height(Length::Fixed(lista::HUECO_FILA)),
            self.boton_pie(
                t("Ajustes de cuentas…", "Account Settings…"),
                None,
                self.pie.intensidad(1),
            ),
        ];

        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(lista::BAJO_CABECERA)),
            lista::grupo(column![cuerpo, pie].into()),
        ];

        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }

    fn fila_cuenta<'a>(&'a self, c: &'a Cuenta, señalada: f32) -> PanelElement<'a> {
        let base_grupo = tema::superficie_en_tarjeta();
        let fondo = if c.actual {
            base_grupo
        } else {
            tema::mezclar(base_grupo, tema::elevado(), señalada)
        };

        let avatar: PanelElement<'a> = if let Some(handle) = &c.foto {
            iced_widget::image(handle.clone())
                .width(Length::Fixed(AVATAR))
                .height(Length::Fixed(AVATAR))
                .content_fit(iced_core::ContentFit::Cover)
                .into()
        } else {
            let (circ_fondo, circ_tinta) = if c.actual {
                let f = tema::acento();
                (f, tinta_avatar_activo(f))
            } else {
                (tema::mezclar(fondo, tema::texto(), 0.10), tema::texto())
            };
            container(
                text(c.iniciales.clone())
                    .size(14.0)
                    .font(control::peso(Weight::Semibold))
                    .color(circ_tinta),
            )
            .width(Length::Fixed(AVATAR))
            .height(Length::Fixed(AVATAR))
            .center_x(Length::Fixed(AVATAR))
            .center_y(Length::Fixed(AVATAR))
            .style(move |_| container::Style {
                background: Some(circ_fondo.into()),
                border: Border {
                    radius: (AVATAR / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        };

        let subtitulo: PanelElement<'a> = if c.actual {
            text(t("Sesión actual", "Current session"))
                .size(11.0)
                .color(tema::texto2())
                .into()
        } else if c.abierta {
            let color_punto = color_punto_sesion(fondo);
            let punto = container(Space::new())
                .width(Length::Fixed(PUNTO_SESION))
                .height(Length::Fixed(PUNTO_SESION))
                .style(move |_| container::Style {
                    background: Some(color_punto.into()),
                    border: Border {
                        radius: (PUNTO_SESION / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            row![
                punto,
                Space::new().width(Length::Fixed(6.0)),
                text(t("Sesión abierta", "Logged in"))
                    .size(11.0)
                    .color(tema::texto2()),
            ]
            .align_y(Vertical::Center)
            .into()
        } else {
            text(t("Cambiar de cuenta…", "Switch account…"))
                .size(11.0)
                .color(tema::texto2())
                .into()
        };

        let textos = column![
            text(lista::recortar(&c.nombre, ANCHO_NOMBRE, 14.0))
                .size(14.0)
                .font(control::peso(if c.actual {
                    Weight::Semibold
                } else {
                    Weight::Medium
                }))
                .color(tema::texto()),
            subtitulo,
        ];

        let derecha: PanelElement<'a> = if c.actual {
            match icono::propio("comprobado") {
                Some(ic) => icono::ver_teñido_propio(&ic, 16.0, color_comprobado(fondo)),
                None => Space::new().width(Length::Fixed(16.0)).into(),
            }
        } else {
            match icono::propio("chevron-derecha") {
                Some(ic) => icono::ver_teñido_propio(&ic, 14.0, tema::texto2()),
                None => Space::new().width(Length::Fixed(14.0)).into(),
            }
        };

        let contenido = row![
            avatar,
            Space::new().width(Length::Fixed(12.0)),
            textos,
            Space::new().width(Length::Fill),
            derecha,
        ]
        .align_y(Vertical::Center);

        container(contenido)
            .width(Length::Fixed(lista::BALDOSA))
            .height(Length::Fixed(lista::FILA))
            .center_y(Length::Fixed(lista::FILA))
            .padding(iced_core::Padding {
                left: 6.0,
                right: 12.0,
                ..Default::default()
            })
            .style(move |_| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn boton_pie<'a>(
        &self,
        etiqueta: &'a str,
        icono_nombre: Option<&str>,
        realzado: f32,
    ) -> PanelElement<'a> {
        let mut fila = row![].align_y(Vertical::Center);
        if let Some(nombre) = icono_nombre
            && let Some(ic) = icono::propio(nombre)
        {
            fila = fila
                .push(icono::ver_teñido_propio(&ic, 16.0, tema::texto()))
                .push(Space::new().width(Length::Fixed(8.0)));
        }
        fila = fila.push(
            text(etiqueta)
                .size(tema::T_CUERPO)
                .font(control::peso(Weight::Semibold))
                .color(tema::texto()),
        );

        container(fila)
            .width(Length::Fixed(lista::BALDOSA))
            .height(Length::Fixed(BOTON_PIE))
            .center_x(Length::Fixed(lista::BALDOSA))
            .center_y(Length::Fixed(BOTON_PIE))
            .style(move |_| container::Style {
                background: Some(control::baldosa(realzado).into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD_3: &str = "claudia:x:1002:1002:Claudia Pérez,,,:/home/claudia:/bin/bash\n\
                            eve:x:1000:1000:Evelyn,,,:/home/eve:/bin/bash\n\
                            alex:x:1001:1001:Alex Martín,,,:/home/alex:/bin/bash\n";

    #[test]
    fn ordena_actual_abiertas_y_cerradas() {
        let u = Usuarios::desde_passwd(PASSWD_3, "eve", &[1001]);
        assert_eq!(u.cuentas.len(), 3);
        assert_eq!(u.cuentas[0].cuenta, "eve");
        assert!(u.cuentas[0].actual);
        assert_eq!(u.cuentas[1].cuenta, "alex");
        assert!(u.cuentas[1].abierta);
        assert_eq!(u.cuentas[2].cuenta, "claudia");
        assert!(!u.cuentas[2].abierta);
    }

    #[test]
    fn clics_en_cuentas_y_botones_del_pie() {
        let mut u = Usuarios::desde_passwd(PASSWD_3, "eve", &[1001]);
        let cx = lista::MARGEN + control::GRUPO + lista::BALDOSA / 2.0;
        let y0 = u.y_lista();

        // La cuenta actual no ejecuta acción ni se resalta.
        assert_eq!(u.pulsar(cx, y0 + lista::FILA / 2.0), None);

        // Segunda fila: sesión abierta (alex).
        let y1 = y0 + (lista::FILA + lista::HUECO_FILA) + lista::FILA / 2.0;
        assert_eq!(
            u.pulsar(cx, y1),
            Some(Accion::Lanzar("bookos-cambiar-usuario 'alex'".into()))
        );

        // Tercera fila: sesión cerrada (claudia).
        let y2 = y0 + 2.0 * (lista::FILA + lista::HUECO_FILA) + lista::FILA / 2.0;
        assert_eq!(
            u.pulsar(cx, y2),
            Some(Accion::Lanzar("bookos-cambiar-usuario 'claudia'".into()))
        );

        // Pie: Ventana de inicio de sesión y Ajustes de cuentas.
        let yp = u.y_pie();
        assert_eq!(
            u.pulsar(cx, yp + BOTON_PIE / 2.0),
            Some(Accion::Lanzar("bookos-cambiar-usuario".into()))
        );
        assert_eq!(
            u.pulsar(cx, yp + BOTON_PIE + lista::HUECO_FILA + BOTON_PIE / 2.0),
            Some(Accion::Lanzar("bookos-settings --page cuentas".into()))
        );
    }

    #[test]
    fn navegacion_con_teclado_y_scroll_con_muchas_cuentas() {
        use crate::TeclaPulsada as T;
        let mut passwd = String::from("eve:x:1000:1000:Evelyn,,,:/home/eve:/bin/bash\n");
        for i in 1..=7 {
            let uid = 1000 + i;
            passwd.push_str(&format!(
                "u{i}:x:{uid}:{uid}:Usuario {i},,,:/home/u{i}:/bin/bash\n"
            ));
        }
        let mut u = Usuarios::desde_passwd(&passwd, "eve", &[]);
        assert!(u.maximo() > 0.0);

        assert!(matches!(u.tecla(T::Abajo), Tecla::Consumida));
        assert_eq!(u.foco, Some(Foco::Cuenta(1)));

        // Bajar hasta la última cuenta desplaza la ventana.
        for _ in 0..6 {
            assert!(matches!(u.tecla(T::Abajo), Tecla::Consumida));
        }
        assert_eq!(u.foco, Some(Foco::Cuenta(7)));
        assert!(u.desplazamiento > 0.0);

        assert!(matches!(u.tecla(T::Fin), Tecla::Consumida));
        assert_eq!(u.foco, Some(Foco::Pie(1)));
        assert!(matches!(
            u.tecla(T::Intro),
            Tecla::Hacer(Accion::Lanzar(cmd)) if cmd == "bookos-settings --page cuentas"
        ));

        assert!(matches!(u.tecla(T::Inicio), Tecla::Consumida));
        assert_eq!(u.foco, Some(Foco::Cuenta(1)));
        assert_eq!(u.desplazamiento, 0.0);
        assert!(matches!(
            u.tecla(T::Intro),
            Tecla::Hacer(Accion::Lanzar(cmd)) if cmd.starts_with("bookos-cambiar-usuario 'u")
        ));

        assert!(matches!(u.tecla(T::Escape), Tecla::Cerrar));
        assert!(!u.accesible().is_empty());
    }

    #[test]
    fn contraste_hig_en_claro_y_oscuro() {
        let antes = tema::actual();
        for t_tema in [tema::Tema::Oscuro, tema::Tema::Claro] {
            tema::aplicar(t_tema);
            let superficie = tema::superficie_en_tarjeta();
            let baldosa = control::baldosa(0.0);
            assert!(tema::contraste(tema::texto(), superficie) >= 4.5);
            assert!(tema::contraste(tema::texto2(), superficie) >= 4.5);
            assert!(tema::contraste(tema::texto(), baldosa) >= 4.5);
            assert!(tema::contraste(tema::texto(), tema::elevado()) >= 4.5);

            let acento = tema::acento();
            assert!(tema::contraste(tinta_avatar_activo(acento), acento) >= 4.5);
            assert!(tema::contraste(color_comprobado(superficie), superficie) >= 3.0);
            assert!(tema::contraste(color_punto_sesion(superficie), superficie) >= 3.0);
            assert!(tema::contraste(color_punto_sesion(tema::elevado()), tema::elevado()) >= 3.0);
        }
        tema::aplicar(antes);
    }

    /// `BOOKOS_USUARIOS_PNG=/tmp/bookos-usuarios` vuelca la tarjeta en oscuro y claro a 2×.
    #[test]
    fn volcar_png() {
        let Ok(prefijo) = std::env::var("BOOKOS_USUARIOS_PNG") else {
            return;
        };
        let antes = tema::actual();
        for (sufijo, t_tema) in [("oscuro", tema::Tema::Oscuro), ("claro", tema::Tema::Claro)] {
            tema::aplicar(t_tema);
            let config = crate::Config {
                tema: t_tema,
                ..Default::default()
            };
            let mut shell = crate::Shell::con_config(1646, 2.0, config);
            let mut u = Usuarios::desde_passwd(PASSWD_3, "eve", &[1001]);
            let y1 = u.y_lista() + (lista::FILA + lista::HUECO_FILA) + lista::FILA / 2.0;
            u.puntero(Some((lista::MARGEN + control::GRUPO + 40.0, y1)));
            std::thread::sleep(std::time::Duration::from_millis(260));
            shell.abrir(super::super::Emergente::Usuarios(u));
            let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_emergente(&mut buf);
            for p in buf.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            image::save_buffer(
                format!("{prefijo}-{sufijo}.png"),
                &buf,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .expect("se escribe el PNG");
        }
        tema::aplicar(antes);
    }
}
