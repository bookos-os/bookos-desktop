//! El menú contextual de un icono del dock: el del clic derecho.
//!
//! Es la versión corta del menú del gestor de tareas de Plasma. De sus siete
//! entradas se quedan las tres que hacen algo aquí —abrir otra ventana, fijar o
//! soltar, cerrar—; las demás configuran el propio applet, que en este dock no
//! existe como applet configurable.
//!
//! Comparte las filas con el menú del logo: son el mismo control y el mismo
//! hover en acento sólido, y duplicarlos habría dejado dos menús que se parecen
//! pero no se comportan igual.

use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::escritorio::NuevoElemento;
use crate::tema;
use crate::view::PanelElement;

use super::menu::{DIVISOR, FILA, HUECO_ICONO, ICONO, MARGEN as MARGEN_FILA, caja, divisor, fila};
use super::{Ancla, Tecla};

/// Más ancho que el del logo: "Desanclar del dock" con su icono no cabía en 210.
const ANCHO: f32 = 230.0;
const BORDE_SUP: f32 = 4.0;
const BORDE_INF: f32 = 4.0;

/// Lo que sabe el menú del icono sobre el que se abrió.
pub struct Objetivo {
    pub app_id: String,
    pub exec: String,
    pub icono: String,
    pub etiqueta: String,
    pub anclada: bool,
    pub abierta: bool,
    /// Centro del icono en coordenadas lógicas **relativas al dock**, para
    /// sacar el menú justo encima de él.
    pub x: f32,
}

enum Entrada {
    Item {
        etiqueta: String,
        icono: &'static str,
        accion: Accion,
    },
    Divisor,
    /// Una fila que despliega a su derecha otra lista al pasar el puntero.
    Submenu {
        etiqueta: String,
        icono: &'static str,
        hijos: Vec<(String, &'static str, Accion)>,
    },
}

/// Hueco entre el menú y su submenú. Cuenta como zona del submenú al mover el
/// puntero: si no, cruzarlo en diagonal cerraba el submenú justo antes de
/// llegar.
const HUECO_SUBMENU: f32 = 2.0;

pub struct MenuDock {
    ventana: bool,
    /// Ancho del menú: fijo en el de ventana y en el del dock, ajustado al
    /// texto más largo en el del escritorio.
    ancho: f32,
    entradas: Vec<Entrada>,
    señalada: tema::Realce,
    x: f32,
    /// Punto lógico donde anclar la esquina superior izquierda, si lo hay.
    /// `None` es el menú de ventana centrado de toda la vida.
    punto: Option<(f32, f32)>,
    /// La entrada de tipo `Submenu` que tiene el submenú desplegado.
    abierto: Option<usize>,
    /// La fila del submenú bajo el puntero o el teclado.
    señalada_sub: tema::Realce,
    /// Las flechas mueven la lista del submenú y no la principal.
    foco_sub: bool,
    ancho_sub: f32,
}

impl MenuDock {
    // 0.1.0
    pub fn es_ventana(&self) -> bool {
        self.ventana
    }
    // 0.1.0
    pub fn ventana(opciones: Vec<(String, Accion)>) -> Self {
        Self::construir(opciones, None)
    }

    /// El menú del clic derecho sobre el escritorio, anclado al punto.
    ///
    /// Con las filas compactas del dock y del logo, con icono, y no con las de
    /// 48 px del menú de ventana: aquel está pensado para leerse centrado en
    /// pantalla, y junto al puntero salía una losa de 310 px para una sola
    /// entrada. `None` en `opciones` es un divisor.
    // 0.1.1
    pub fn escritorio(
        opciones: Vec<Option<(String, &'static str, Accion)>>,
        punto: (f32, f32),
    ) -> Self {
        let texto = opciones
            .iter()
            .flatten()
            .map(|(etiqueta, _, _)| crate::widget::ancho_de(etiqueta, tema::T_CUERPO))
            .fold(0.0, f32::max);
        // Lo que rodea al texto en `menu::fila`: margen y relleno a cada lado,
        // icono y su hueco. Los 16 de propina son el aire que deja el logo a la
        // derecha de su entrada más larga; sin él el texto toca el borde.
        let ancho = (MARGEN_FILA * 2.0 + 22.0 + ICONO + HUECO_ICONO + texto + 16.0)
            .ceil()
            .max(160.0);
        // La entrada de crear carpeta se despliega en el submenú «Crear» con
        // los demás tipos. Los tipos viven aquí y no en quien arma las
        // opciones: es una forma de dibujarla, no una decisión de qué ofrecer.
        let mut ancho_sub: f32 = 0.0;
        let entradas: Vec<Entrada> = opciones
            .into_iter()
            .map(|opcion| match opcion {
                Some((etiqueta, icono, Accion::EscritorioNuevo(NuevoElemento::Carpeta))) => {
                    let hijos = vec![
                        (
                            "Carpeta".to_string(),
                            "carpeta-nueva",
                            NuevoElemento::Carpeta,
                        ),
                        (
                            "Documento de texto".to_string(),
                            "editar",
                            NuevoElemento::Texto,
                        ),
                        (
                            "Script de shell".to_string(),
                            "terminal",
                            NuevoElemento::Script,
                        ),
                        ("Archivo vacío".to_string(), "abrir", NuevoElemento::Vacio),
                    ]
                    .into_iter()
                    .map(|(e, i, t)| (e, i, Accion::EscritorioNuevo(t)))
                    .collect::<Vec<_>>();
                    ancho_sub = hijos
                        .iter()
                        .map(|(e, ..)| crate::widget::ancho_de(e, tema::T_CUERPO))
                        .fold(0.0, f32::max);
                    Entrada::Submenu {
                        etiqueta,
                        icono,
                        hijos,
                    }
                }
                Some((etiqueta, icono, accion)) => Entrada::Item {
                    etiqueta,
                    icono,
                    accion,
                },
                None => Entrada::Divisor,
            })
            .collect();
        let ancho_sub = if ancho_sub > 0.0 {
            (MARGEN_FILA * 2.0 + 22.0 + ICONO + HUECO_ICONO + ancho_sub + 16.0).ceil()
        } else {
            0.0
        };
        Self {
            ventana: false,
            ancho,
            entradas,
            señalada: tema::Realce::nuevo(),
            x: 0.0,
            punto: Some(punto),
            abierto: None,
            señalada_sub: tema::Realce::nuevo(),
            foco_sub: false,
            ancho_sub,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- ancho guardado en el menú
    fn construir(opciones: Vec<(String, Accion)>, punto: Option<(f32, f32)>) -> Self {
        Self {
            ancho: 310.0,
            entradas: opciones
                .into_iter()
                .map(|(etiqueta, accion)| Entrada::Item {
                    etiqueta,
                    icono: "chevron-derecha",
                    accion,
                })
                .collect(),
            señalada: tema::Realce::nuevo(),
            x: 0.0,
            ventana: true,
            punto,
            abierto: None,
            señalada_sub: tema::Realce::nuevo(),
            foco_sub: false,
            ancho_sub: 0.0,
        }
    }
    // 0.1.0
    // modified 0.1.1 -- «Abrir» si la aplicación está cerrada
    pub fn new(objetivo: Objetivo) -> Self {
        // «Nueva ventana» solo tiene sentido si ya hay una: sobre una
        // aplicación cerrada lo que hace la entrada es abrirla, y así se llama.
        let mut entradas = vec![Entrada::Item {
            etiqueta: if objetivo.abierta {
                "Abrir una nueva ventana".into()
            } else {
                "Abrir".into()
            },
            icono: "ventana-nueva",
            accion: Accion::Lanzar(objetivo.exec.clone()),
        }];
        entradas.push(Entrada::Item {
            etiqueta: if objetivo.anclada {
                "Desanclar del dock".into()
            } else {
                "Anclar al dock".into()
            },
            // modificado en la 0.2.0 -- macizo si ya está anclada: se lee el estado sin leer la etiqueta.
            icono: if objetivo.anclada {
                "fijar-anclado"
            } else {
                "fijar"
            },
            accion: Accion::Anclar {
                app_id: objetivo.app_id.clone(),
                exec: objetivo.exec.clone(),
                icono: objetivo.icono.clone(),
                fijar: None,
            },
        });
        // Cerrar solo aparece si hay algo que cerrar: una entrada permanente
        // que la mitad de las veces no hace nada enseña a desconfiar del menú.
        if objetivo.abierta {
            entradas.push(Entrada::Divisor);
            entradas.push(Entrada::Item {
                etiqueta: "Cerrar".into(),
                icono: "cerrar",
                accion: Accion::Cerrar {
                    app_id: objetivo.app_id.clone(),
                },
            });
        }
        Self {
            ventana: false,
            ancho: ANCHO,
            entradas,
            señalada: tema::Realce::nuevo(),
            x: objetivo.x,
            punto: None,
            abierto: None,
            señalada_sub: tema::Realce::nuevo(),
            foco_sub: false,
            ancho_sub: 0.0,
        }
    }

    // 0.1.0
    // modified 0.2.0 -- reserva sitio para el submenú, abierto o no
    pub fn size(&self) -> (f32, f32) {
        let (ancho, alto) = (self.ancho(), self.alto_principal());
        let Some((i, hijos)) = self.submenu() else {
            return (ancho, alto);
        };
        // Se reserva con el submenú cerrado también: crecer al abrirlo
        // desplazaría el menú entero si estaba junto al borde derecho, justo
        // bajo el puntero.
        let alto_sub = self.y_de(i) - BORDE_SUP + Self::alto_sub(hijos.len());
        (ancho + HUECO_SUBMENU + self.ancho_sub, alto.max(alto_sub))
    }

    // 0.2.0
    fn alto_entrada(&self, entrada: &Entrada) -> f32 {
        match entrada {
            Entrada::Divisor => DIVISOR,
            _ => self.alto_fila(),
        }
    }

    // 0.2.0
    fn alto_principal(&self) -> f32 {
        let filas: f32 = self.entradas.iter().map(|e| self.alto_entrada(e)).sum();
        filas + BORDE_SUP + BORDE_INF
    }

    /// La `y` donde empieza la entrada `i`.
    // 0.2.0
    fn y_de(&self, i: usize) -> f32 {
        BORDE_SUP
            + self.entradas[..i]
                .iter()
                .map(|e| self.alto_entrada(e))
                .sum::<f32>()
    }

    // 0.2.0
    fn alto_sub(n: usize) -> f32 {
        n as f32 * FILA + BORDE_SUP + BORDE_INF
    }

    /// La entrada con submenú y sus hijos, si el menú tiene una.
    // 0.2.0
    fn submenu(&self) -> Option<(usize, &[(String, &'static str, Accion)])> {
        self.entradas.iter().enumerate().find_map(|(i, e)| match e {
            Entrada::Submenu { hijos, .. } => Some((i, hijos.as_slice())),
            _ => None,
        })
    }

    /// Qué fila del submenú abierto cae en el punto. Cuenta el hueco con el
    /// menú, pero solo a la altura del propio submenú.
    // 0.2.0
    fn fila_sub_en(&self, x: f32, y: f32) -> Option<usize> {
        let (i, hijos) = self.submenu().filter(|(i, _)| self.abierto == Some(*i))?;
        let arriba = self.y_de(i) - BORDE_SUP;
        if x < self.ancho() || x > self.ancho() + HUECO_SUBMENU + self.ancho_sub {
            return None;
        }
        let dentro = y - arriba - BORDE_SUP;
        if dentro < 0.0 || y > arriba + Self::alto_sub(hijos.len()) {
            return None;
        }
        Some(((dentro / FILA) as usize).min(hijos.len() - 1))
    }

    // 0.1.0
    // modified 0.1.1 -- ancho guardado en el menú
    fn ancho(&self) -> f32 {
        self.ancho
    }
    // 0.1.0
    fn alto_fila(&self) -> f32 {
        if self.ventana { 48.0 } else { FILA }
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        if let Some((x, y)) = self.punto {
            return Ancla::Punto { x, y };
        }
        if self.ventana {
            return Ancla::Centrada;
        }
        Ancla::SobreElDock { x: self.x }
    }

    // 0.1.0
    // modified 0.1.1 -- la fila de muestras de color se puede pulsar
    // modified 0.2.0 -- también las filas con submenú
    fn entrada_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho() {
            return None;
        }
        let mut cursor = BORDE_SUP;
        for (i, entrada) in self.entradas.iter().enumerate() {
            let alto = self.alto_entrada(entrada);
            if y >= cursor && y < cursor + alto {
                return (!matches!(entrada, Entrada::Divisor)).then_some(i);
            }
            cursor += alto;
        }
        None
    }

    // 0.1.0
    // modified 0.1.1 -- la fila de muestras no se realza entera
    // modified 0.2.0 -- pasar sobre una fila con submenú lo abre; sobre otra, lo cierra
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((x, y)) = punto else {
            return self.señalada.señalar(None) | self.señalada_sub.señalar(None);
        };
        if let Some(j) = self.fila_sub_en(x, y) {
            // Mientras se recorre el submenú su fila madre sigue encendida,
            // como en macOS: es lo que dice de dónde cuelga.
            let madre = self.abierto;
            self.foco_sub = false;
            return self.señalada.señalar(madre) | self.señalada_sub.señalar(Some(j));
        }
        let Some(i) = self.entrada_en(x, y) else {
            // Fuera de toda fila: no se cierra nada. Cruzar el borde del menú
            // hacia el submenú pasa por un píxel así, y cerrarlo ahí lo
            // volvería imposible de alcanzar.
            return false;
        };
        let abierto = matches!(self.entradas[i], Entrada::Submenu { .. }).then_some(i);
        let cambia = self.abierto != abierto;
        self.abierto = abierto;
        let fila = matches!(
            self.entradas[i],
            Entrada::Item { .. } | Entrada::Submenu { .. }
        )
        .then_some(i);
        self.señalada.señalar(fila) | self.señalada_sub.señalar(None) | cambia
    }

    // 0.1.0
    // modified 0.2.0 -- también la fila del submenú
    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.señalada_sub.animando()
    }

    // 0.1.0
    // modified 0.1.1 -- elegir o quitar el color de una carpeta del escritorio
    // modified 0.2.0 -- pulsar una fila con submenú lo abre; una del submenú actúa
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(j) = self.fila_sub_en(x, y) {
            return self.submenu().map(|(_, hijos)| hijos[j].2.clone());
        }
        let i = self.entrada_en(x, y)?;
        match &self.entradas[i] {
            Entrada::Item { accion, .. } => Some(accion.clone()),
            Entrada::Divisor => None,
            Entrada::Submenu { .. } => {
                // Para quien no usa el ratón con hover (táctil): el toque
                // abre el submenú, y el siguiente elige.
                self.abierto = Some(i);
                None
            }
        }
    }

    // 0.1.0
    // modified 0.2.0 -- Derecha abre el submenú, Izquierda y Esc lo cierran
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        if self.foco_sub
            && let Some((_, hijos)) = self.submenu()
        {
            let n = hijos.len();
            let actual = self.señalada_sub.actual();
            match tecla {
                T::Arriba => {
                    self.señalada_sub
                        .señalar(Some(actual.map_or(n - 1, |a| (a + n - 1) % n)));
                    return Tecla::Consumida;
                }
                T::Abajo | T::Tabulador => {
                    self.señalada_sub
                        .señalar(Some(actual.map_or(0, |a| (a + 1) % n)));
                    return Tecla::Consumida;
                }
                T::Intro => {
                    return match actual {
                        Some(j) => Tecla::Hacer(hijos[j].2.clone()),
                        None => Tecla::Consumida,
                    };
                }
                T::Izquierda | T::Escape => {
                    self.foco_sub = false;
                    self.abierto = None;
                    self.señalada_sub.señalar(None);
                    return Tecla::Consumida;
                }
                _ => {}
            }
        }
        let sobre_submenu = self
            .señalada
            .actual()
            .filter(|&i| matches!(self.entradas.get(i), Some(Entrada::Submenu { .. })));
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Derecha | T::Intro if sobre_submenu.is_some() => {
                self.abierto = sobre_submenu;
                self.foco_sub = true;
                self.señalada_sub.señalar(Some(0));
                Tecla::Consumida
            }
            T::Arriba | T::Abajo | T::Tabulador => {
                let indices: Vec<_> = self
                    .entradas
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| {
                        matches!(e, Entrada::Item { .. } | Entrada::Submenu { .. }).then_some(i)
                    })
                    .collect();
                if indices.is_empty() {
                    return Tecla::Consumida;
                }
                let actual = self
                    .señalada
                    .actual()
                    .and_then(|a| indices.iter().position(|i| *i == a));
                let n = if tecla == T::Arriba {
                    actual.map_or(indices.len() - 1, |a| {
                        (a + indices.len() - 1) % indices.len()
                    })
                } else {
                    actual.map_or(0, |a| (a + 1) % indices.len())
                };
                self.señalada.señalar(Some(indices[n]));
                // Moverse a otra fila cierra el submenú que hubiera.
                self.abierto = None;
                Tecla::Consumida
            }
            T::Intro => {
                if let Some(Entrada::Item { accion, .. }) =
                    self.señalada.actual().and_then(|i| self.entradas.get(i))
                {
                    Tecla::Hacer(accion.clone())
                } else {
                    Tecla::Consumida
                }
            }
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- pinta la fila de muestras de color
    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(Length::Fixed(BORDE_SUP))];
        for (i, entrada) in self.entradas.iter().enumerate() {
            col = col.push(match entrada {
                Entrada::Item {
                    etiqueta, icono, ..
                } => {
                    if self.ventana {
                        let realce = self.señalada.intensidad(i);
                        container(
                            text(super::recortar_texto(etiqueta, 270.0, 14.0))
                                .size(14)
                                .color(tema::texto()),
                        )
                        .padding([14, 20])
                        .width(self.ancho())
                        .height(self.alto_fila())
                        .style(move |_| container::Style {
                            background: Some(tema::alfa(tema::acento(), realce * 0.10).into()),
                            border: Border {
                                radius: 13.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .into()
                    } else {
                        fila(etiqueta, None, Some(icono), self.señalada.intensidad(i))
                    }
                }
                Entrada::Divisor => divisor(),
                Entrada::Submenu {
                    etiqueta, icono, ..
                } => fila(
                    etiqueta,
                    Some("›"),
                    Some(icono),
                    self.señalada.intensidad(i),
                ),
            });
        }
        col = col.push(Space::new().height(Length::Fixed(BORDE_INF)));

        let principal = caja(col.into(), self.ancho());
        let (Some((i, hijos)), Some(_)) = (self.submenu(), self.abierto) else {
            return principal;
        };
        let mut filas = column![Space::new().height(Length::Fixed(BORDE_SUP))];
        for (j, (etiqueta, icono, _)) in hijos.iter().enumerate() {
            filas = filas.push(fila(
                etiqueta,
                None,
                Some(icono),
                self.señalada_sub.intensidad(j),
            ));
        }
        filas = filas.push(Space::new().height(Length::Fixed(BORDE_INF)));
        // Alineado con su fila madre: la primera del submenú queda a su altura.
        let lateral = column![
            Space::new().height(Length::Fixed(self.y_de(i) - BORDE_SUP)),
            caja(filas.into(), self.ancho_sub)
        ];
        row![
            column![principal].width(Length::Fixed(self.ancho())),
            Space::new().width(Length::Fixed(HUECO_SUBMENU)),
            lateral
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teclado_selecciona_el_destino_y_escape_cancela() {
        let mut menu = MenuDock::ventana(vec![
            ("Siempre encima".into(), Accion::VentanaEncima),
            ("Mover a Escritorio 2".into(), Accion::VentanaEscritorio(1)),
        ]);
        menu.tecla(crate::TeclaPulsada::Abajo);
        menu.tecla(crate::TeclaPulsada::Abajo);
        assert!(matches!(
            menu.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::VentanaEscritorio(1))
        ));
        assert!(matches!(
            menu.tecla(crate::TeclaPulsada::Escape),
            Tecla::Cerrar
        ));
    }

    fn objetivo(anclada: bool, abierta: bool) -> Objetivo {
        Objetivo {
            app_id: "konsole".into(),
            exec: "konsole".into(),
            icono: "utilities-terminal".into(),
            etiqueta: "Terminal".into(),
            anclada,
            abierta,
            x: 100.0,
        }
    }

    /// Sin ventanas no hay nada que cerrar, y la entrada no debe salir.
    #[test]
    fn cerrar_solo_con_ventana_abierta() {
        let cerrado = MenuDock::new(objetivo(true, false));
        assert_eq!(cerrado.entradas.len(), 2);
        let abierto = MenuDock::new(objetivo(true, true));
        assert_eq!(abierto.entradas.len(), 4, "faltan el divisor y Cerrar");
    }

    /// La segunda entrada dice lo contrario de lo que ya está: si está fijada,
    /// quitarla.
    #[test]
    fn la_entrada_de_fijar_cambia_con_el_estado() {
        let m = MenuDock::new(objetivo(true, false));
        let Entrada::Item { etiqueta, .. } = &m.entradas[1] else {
            panic!("la segunda entrada no es un item");
        };
        assert_eq!(etiqueta, "Desanclar del dock");
        let m = MenuDock::new(objetivo(false, false));
        let Entrada::Item { etiqueta, .. } = &m.entradas[1] else {
            panic!("la segunda entrada no es un item");
        };
        assert_eq!(etiqueta, "Anclar al dock");
    }

    fn con_crear() -> MenuDock {
        MenuDock::escritorio(
            vec![
                Some((
                    "Crear".into(),
                    "vista-anadir",
                    Accion::EscritorioNuevo(NuevoElemento::Carpeta),
                )),
                None,
                Some(("Cambiar fondo…".into(), "fondo", Accion::AbrirCaptura)),
            ],
            (0.0, 0.0),
        )
    }

    /// Pasar sobre «Crear» abre su lista a la derecha; sobre otra fila, la
    /// cierra; y cruzar el hueco entre las dos no la cierra.
    #[test]
    fn el_submenu_se_abre_al_pasar_y_se_cierra_al_salir() {
        let mut m = con_crear();
        let y = BORDE_SUP + FILA / 2.0;
        let ancho = m.ancho();
        assert!(m.size().0 > ancho, "reserva el sitio del submenú");
        m.puntero(Some((20.0, y)));
        assert_eq!(m.abierto, Some(0));
        m.puntero(Some((ancho + 1.0, y)));
        assert_eq!(m.abierto, Some(0), "el hueco no lo cierra");
        m.puntero(Some((20.0, BORDE_SUP + FILA + DIVISOR + FILA / 2.0)));
        assert_eq!(m.abierto, None);
    }

    /// Cada fila del submenú crea lo suyo, a su altura.
    #[test]
    fn las_filas_del_submenu_crean_cada_tipo() {
        let mut m = con_crear();
        m.puntero(Some((20.0, BORDE_SUP + FILA / 2.0)));
        let x = m.ancho() + HUECO_SUBMENU + 20.0;
        let y = |j: f32| BORDE_SUP + BORDE_SUP + FILA * j + FILA / 2.0;
        assert_eq!(
            m.pulsar(x, y(0.0)),
            Some(Accion::EscritorioNuevo(NuevoElemento::Carpeta))
        );
        assert_eq!(
            m.pulsar(x, y(1.0)),
            Some(Accion::EscritorioNuevo(NuevoElemento::Texto))
        );
        assert_eq!(
            m.pulsar(x, y(3.0)),
            Some(Accion::EscritorioNuevo(NuevoElemento::Vacio))
        );
    }

    /// Con teclado: Abajo hasta «Crear», Derecha entra, Intro elige.
    #[test]
    fn el_submenu_se_maneja_con_teclado() {
        use crate::TeclaPulsada as T;
        let mut m = con_crear();
        m.tecla(T::Abajo);
        m.tecla(T::Derecha);
        m.tecla(T::Abajo);
        assert!(matches!(
            m.tecla(T::Intro),
            Tecla::Hacer(Accion::EscritorioNuevo(NuevoElemento::Texto))
        ));
        assert!(matches!(m.tecla(T::Izquierda), Tecla::Consumida));
        assert!(matches!(m.tecla(T::Escape), Tecla::Cerrar));
    }

    /// El hueco del divisor no activa nada.
    #[test]
    fn el_divisor_no_se_pulsa() {
        let mut m = MenuDock::new(objetivo(true, true));
        assert!(m.pulsar(20.0, BORDE_SUP + FILA * 2.0 + 1.0).is_none());
    }
}
