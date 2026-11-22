//! Los diálogos del escritorio que salen del menú contextual: pegar (acceso
//! directo o copia), renombrar, información y vista rápida.
//!
//! Siguen el diálogo del HIG §9: 310 de ancho (420 la vista rápida, que es
//! contenido amplio), radio 26, relleno `28 24 20`, título 17/700 centrado,
//! botones de 48 con radio 14 y cancelar a la izquierda. La sombra es la del
//! diálogo de color de carpeta, rasterizada una vez al abrir.
//!
//! Todas las medidas son fijas y se calculan al construir: el compositor
//! rehace la superficie si cambia de tamaño, y eso rearrancaría la entrada.
//! Por eso el error de «Renombrar» tiene su renglón reservado aunque no haya
//! error, y la vista rápida decide su alto con la imagen ya leída.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, stack, text};

use super::color_carpeta::{SOMBRA, sombra_dialogo};
use super::{Ancla, FilaAccesible, Tecla, t};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const ANCHO: f32 = 310.0;
const ANCHO_AMPLIO: f32 = 420.0;
const LATERAL: f32 = 24.0;
const ARRIBA: f32 = 28.0;
const ABAJO: f32 = 20.0;
const BOTON: f32 = 48.0;
const HUECO_BOTON: f32 = 10.0;
/// Botonera: 12 por encima, como pide el HIG.
const AIRE_BOTONES: f32 = 12.0;
/// Título 17 con interlineado 1,3.
const T_TITULO: f32 = 17.0;
const LH_TITULO: f32 = 22.0;
const T_MENSAJE: f32 = 13.0;
const LH_MENSAJE: f32 = 19.5;
/// Fila de detalle: mínimo 44 dentro del diálogo (la del HIG es 48 en la
/// página de Ajustes; aquí el diálogo es estrecho y lo que manda es la zona
/// táctil de §11).
const FILA_INFO: f32 = 44.0;
const CABECERA_GRUPO: f32 = 12.0 + 18.0 + 8.0;
/// Opción de pegar: icono de 36 y relleno de 12.
const OPCION_ICONO: f32 = 36.0;
const OPCION_RELLENO: f32 = 12.0;
const HUECO_OPCION: f32 = 8.0;
const T_SUB: f32 = 11.0;
const LH_SUB: f32 = 15.0;
/// Campo de texto de renombrar.
const CAMPO: f32 = 44.0;
const ERROR: f32 = 18.0;

/// Qué se puede pulsar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pieza {
    Cancelar,
    Aceptar,
    /// Opción `n` de un diálogo de elegir (pegar).
    Opcion(usize),
    /// El interruptor de una fila de la ficha de información.
    Interruptor,
}

/// Una opción grande con icono, título y explicación.
pub struct Opcion {
    pub icono: &'static str,
    pub titulo: String,
    pub accion: Accion,
}

/// Una fila de la ficha de información.
pub enum Dato {
    Texto(String, String),
    /// Un interruptor con su explicación debajo.
    Interruptor {
        titulo: String,
        detalle: String,
        encendido: bool,
        /// Lo que se pide al cambiarlo; lleva ya el valor nuevo.
        accion: Accion,
    },
}

enum Contenido {
    Elegir {
        mensaje: Vec<String>,
        opciones: Vec<(Opcion, Vec<String>)>,
    },
    Renombrar {
        texto: String,
        /// En caracteres, no en bytes.
        cursor: usize,
        /// Todo seleccionado: lo primero que se escriba lo sustituye.
        todo: bool,
        /// Por qué no se pudo renombrar, debajo del campo.
        error: Option<String>,
    },
    Info {
        icono: Option<Icono>,
        subtitulo: String,
        grupos: Vec<(String, Vec<Dato>)>,
    },
    Vista {
        imagen: Option<(iced_core::image::Handle, f32, f32)>,
        texto: Vec<String>,
        icono: Option<Icono>,
        detalle: String,
    },
}

pub struct Dialogo {
    ancho: f32,
    titulo: Vec<String>,
    contenido: Contenido,
    alto: f32,
    sombra: [Icono; 4],
    señalado: Option<Pieza>,
    /// Botón principal: su etiqueta, o `None` si solo hay «Cancelar» o «Hecho».
    aceptar: Option<String>,
    cancelar: String,
}

/// Renglones de un texto en el ancho interior, como mucho `max`.
fn renglones(texto: &str, ancho: f32, tamaño: f32, max: usize) -> Vec<String> {
    crate::escritorio::lineas(texto, ancho, tamaño, max)
}

impl Dialogo {
    fn interior(&self) -> f32 {
        self.ancho - LATERAL * 2.0
    }

    fn construir(
        ancho: f32,
        titulo: &str,
        contenido: Contenido,
        aceptar: Option<String>,
        cancelar: String,
    ) -> Self {
        let titulo = renglones(titulo, ancho - LATERAL * 2.0, T_TITULO, 2);
        let mut d = Self {
            ancho,
            titulo,
            contenido,
            alto: 0.0,
            sombra: sombra_dialogo(ancho, 100.0),
            señalado: None,
            aceptar,
            cancelar,
        };
        d.alto = d.medir();
        d.sombra = sombra_dialogo(ancho, d.alto);
        d
    }

    /// «¿Cómo quieres pegar …?»: acceso directo o copia.
    // añadido en la 0.2.3
    pub fn pegar(nombres: &[String], origen: &str, peso: u64) -> Self {
        let interior = ANCHO - LATERAL * 2.0;
        let (titulo, a, c) = match nombres {
            [uno] => (
                format!(
                    "{} «{uno}»?",
                    t("¿Cómo quieres pegar", "How do you want to paste")
                ),
                (
                    t("Crear un acceso directo", "Create a shortcut"),
                    t(
                        "Abre el original. Lo que cambies allí se verá aquí.",
                        "Opens the original. Changes there show here.",
                    )
                    .to_string(),
                ),
                (
                    t("Copiar aquí", "Copy here"),
                    format!(
                        "{} {}.",
                        t("Un duplicado independiente de", "An independent copy of"),
                        crate::menus::tamano_legible(peso)
                    ),
                ),
            ),
            varios => (
                format!(
                    "{} {} {}?",
                    t("¿Cómo quieres pegar", "How do you want to paste"),
                    varios.len(),
                    t("elementos", "items")
                ),
                (
                    t("Crear accesos directos", "Create shortcuts"),
                    t(
                        "Uno por elemento; abren los originales.",
                        "One each; they open the originals.",
                    )
                    .to_string(),
                ),
                (
                    t("Copiarlos aquí", "Copy them here"),
                    format!(
                        "{}, {} {}.",
                        t("Duplicados independientes", "Independent copies"),
                        crate::menus::tamano_legible(peso),
                        t("en total", "in total")
                    ),
                ),
            ),
        };
        let mensaje = format!(
            "{} {} · {}",
            t("Está en", "From"),
            origen,
            crate::menus::tamano_legible(peso)
        );
        let opcion = |icono, (titulo, detalle): (&str, String), accion| {
            let ancho_texto = interior - OPCION_RELLENO * 2.0 - OPCION_ICONO - 12.0;
            let lineas = renglones(&detalle, ancho_texto, T_SUB, 3);
            (
                Opcion {
                    icono,
                    titulo: titulo.to_string(),
                    accion,
                },
                lineas,
            )
        };
        Self::construir(
            ANCHO,
            &titulo,
            Contenido::Elegir {
                mensaje: renglones(&mensaje, interior, T_MENSAJE, 3),
                opciones: vec![
                    opcion("abrir", a, Accion::EscritorioPegarComo(true)),
                    opcion("copiar", c, Accion::EscritorioPegarComo(false)),
                ],
            },
            None,
            t("Cancelar", "Cancel").into(),
        )
    }

    /// Renombrar un icono del escritorio.
    // añadido en la 0.2.3
    pub fn renombrar(nombre: &str) -> Self {
        Self::construir(
            ANCHO,
            t("Renombrar", "Rename"),
            Contenido::Renombrar {
                cursor: nombre.chars().count(),
                texto: nombre.to_string(),
                todo: true,
                error: None,
            },
            Some(t("Renombrar", "Rename").into()),
            t("Cancelar", "Cancel").into(),
        )
    }

    /// El renombrado falló: se dice por qué debajo del campo, sin cerrar.
    // añadido en la 0.2.3
    pub fn error(&mut self, motivo: &str) {
        if let Contenido::Renombrar { error, .. } = &mut self.contenido {
            *error = Some(motivo.to_string());
        }
    }

    /// La ficha de información.
    // añadido en la 0.2.3
    pub fn informacion(
        icono: Option<Icono>,
        nombre: &str,
        subtitulo: &str,
        grupos: Vec<(String, Vec<Dato>)>,
    ) -> Self {
        Self::construir(
            ANCHO,
            nombre,
            Contenido::Info {
                icono,
                subtitulo: subtitulo.to_string(),
                grupos,
            },
            None,
            t("Hecho", "Done").into(),
        )
    }

    /// La vista rápida: la imagen (o la primera página de un PDF) o el
    /// principio de un texto; si no, el icono en grande.
    // añadido en la 0.2.3
    pub fn vista(
        nombre: &str,
        detalle: &str,
        imagen: Option<(Vec<u8>, u32, u32)>,
        texto: Option<&str>,
        icono: Option<Icono>,
    ) -> Self {
        let interior = ANCHO_AMPLIO - LATERAL * 2.0;
        // La imagen ocupa el ancho y como mucho 360 de alto: más alta, el
        // diálogo se saldría de una pantalla de portátil.
        let imagen = imagen.map(|(mut rgba, w, alto)| {
            let escala = (interior / w as f32).min(360.0 / alto as f32).min(1.0);
            // Esquinas de miniatura (radio 14 del HIG) quemadas en los píxeles:
            // iced sobre tiny-skia no recorta una imagen con radio.
            redondear(&mut rgba, w, alto, tema::R_CONTROL / escala);
            (
                iced_core::image::Handle::from_rgba(w, alto, rgba),
                w as f32 * escala,
                alto as f32 * escala,
            )
        });
        let texto = texto
            .map(|t| renglones(t, interior - 24.0, 12.0, 16))
            .unwrap_or_default();
        Self::construir(
            ANCHO_AMPLIO,
            nombre,
            Contenido::Vista {
                imagen,
                texto,
                icono,
                detalle: detalle.to_string(),
            },
            None,
            t("Cerrar", "Close").into(),
        )
    }

    fn alto_titulo(&self) -> f32 {
        self.titulo.len() as f32 * LH_TITULO
    }

    /// Alto de la tarjeta (sin sombra), y de paso dónde cae cada cosa.
    fn medir(&self) -> f32 {
        let botonera = AIRE_BOTONES + BOTON + ABAJO;
        let cuerpo = match &self.contenido {
            Contenido::Elegir { mensaje, opciones } => {
                6.0 + mensaje.len() as f32 * LH_MENSAJE
                    + 18.0
                    + opciones
                        .iter()
                        .map(|(_, l)| alto_opcion(l.len()))
                        .sum::<f32>()
                    + HUECO_OPCION * (opciones.len() as f32 - 1.0)
            }
            Contenido::Renombrar { .. } => 18.0 + CAMPO + ERROR,
            Contenido::Info { grupos, .. } => {
                6.0 + LH_MENSAJE
                    + grupos
                        .iter()
                        .map(|(_, datos)| CABECERA_GRUPO + datos.iter().map(alto_dato).sum::<f32>())
                        .sum::<f32>()
            }
            Contenido::Vista {
                imagen,
                texto,
                detalle,
                ..
            } => {
                let _ = detalle;
                6.0 + LH_MENSAJE
                    + 18.0
                    + match (imagen, texto.is_empty()) {
                        (Some((_, _, h)), _) => *h,
                        (None, false) => texto.len() as f32 * 17.0 + 24.0,
                        (None, true) => 96.0,
                    }
            }
        };
        let cabeza = match &self.contenido {
            Contenido::Info { .. } => 64.0 + 12.0,
            _ => 0.0,
        };
        ARRIBA + cabeza + self.alto_titulo() + cuerpo + botonera
    }

    // añadido en la 0.2.3
    pub fn size(&self) -> (f32, f32) {
        (self.ancho + SOMBRA * 2.0, self.alto + SOMBRA * 2.0)
    }

    // añadido en la 0.2.3
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    // añadido en la 0.2.3
    pub fn animando(&self) -> bool {
        false
    }

    /// El velo del diálogo, .45 del HIG.
    // añadido en la 0.2.3
    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    /// Controles expuestos a AT-SPI2 / Orca.
    // añadido en la 0.2.3
    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let mut out = Vec::new();
        let interior = self.interior();
        match &self.contenido {
            Contenido::Elegir { mensaje, opciones } => {
                let mut top =
                    ARRIBA + self.alto_titulo() + 6.0 + mensaje.len() as f32 * LH_MENSAJE + 18.0;
                for (n, (o, lineas)) in opciones.iter().enumerate() {
                    let h = alto_opcion(lineas.len());
                    out.push(FilaAccesible {
                        id: format!("dialogo:opcion:{n}"),
                        rol: accesskit::Role::RadioButton,
                        nombre: o.titulo.clone(),
                        valor: lineas.join(" "),
                        rect: (SOMBRA + LATERAL, SOMBRA + top, interior, h),
                        marcada: Some(self.señalado == Some(Pieza::Opcion(n))),
                        seleccionada: self.señalado == Some(Pieza::Opcion(n)),
                    });
                    top += h + HUECO_OPCION;
                }
            }
            Contenido::Renombrar { texto, error, .. } => {
                let y = SOMBRA + ARRIBA + self.alto_titulo() + 18.0;
                out.push(FilaAccesible {
                    id: "dialogo:campo".into(),
                    rol: accesskit::Role::TextInput,
                    nombre: t("Nombre", "Name").into(),
                    valor: error.clone().unwrap_or_else(|| texto.clone()),
                    rect: (SOMBRA + LATERAL, y, interior, CAMPO),
                    marcada: None,
                    seleccionada: false,
                });
            }
            Contenido::Info { grupos, .. } => {
                let mut top = ARRIBA + 76.0 + self.alto_titulo() + 6.0 + LH_MENSAJE;
                for (_, datos) in grupos {
                    top += CABECERA_GRUPO;
                    for d in datos {
                        let h = alto_dato(d);
                        if let Dato::Interruptor {
                            titulo,
                            encendido,
                            ..
                        } = d
                        {
                            out.push(FilaAccesible {
                                id: "dialogo:interruptor".into(),
                                rol: accesskit::Role::Switch,
                                nombre: titulo.clone(),
                                valor: String::new(),
                                rect: (SOMBRA + LATERAL, SOMBRA + top, interior, h),
                                marcada: Some(*encendido),
                                seleccionada: self.señalado == Some(Pieza::Interruptor),
                            });
                        }
                        top += h;
                    }
                }
            }
            Contenido::Vista { .. } => {}
        }
        let y_bot = SOMBRA + self.alto - ABAJO - BOTON;
        if let Some(aceptar) = &self.aceptar {
            let medio = (interior - HUECO_BOTON) / 2.0;
            out.push(FilaAccesible {
                id: "dialogo:cancelar".into(),
                rol: accesskit::Role::Button,
                nombre: self.cancelar.clone(),
                valor: String::new(),
                rect: (SOMBRA + LATERAL, y_bot, medio, BOTON),
                marcada: None,
                seleccionada: self.señalado == Some(Pieza::Cancelar),
            });
            out.push(FilaAccesible {
                id: "dialogo:aceptar".into(),
                rol: accesskit::Role::Button,
                nombre: aceptar.clone(),
                valor: String::new(),
                rect: (SOMBRA + LATERAL + medio + HUECO_BOTON, y_bot, medio, BOTON),
                marcada: None,
                seleccionada: self.señalado == Some(Pieza::Aceptar),
            });
        } else {
            out.push(FilaAccesible {
                id: "dialogo:cancelar".into(),
                rol: accesskit::Role::Button,
                nombre: self.cancelar.clone(),
                valor: String::new(),
                rect: (SOMBRA + LATERAL, y_bot, interior, BOTON),
                marcada: None,
                seleccionada: self.señalado == Some(Pieza::Cancelar),
            });
        }
        out
    }

    /// Qué pieza hay en el punto, en coordenadas de la superficie.
    fn pieza_en(&self, x: f32, y: f32) -> Option<Pieza> {
        let (x, y) = (x - SOMBRA - LATERAL, y - SOMBRA);
        let interior = self.interior();
        if !(0.0..=interior).contains(&x) {
            return None;
        }
        let botones = self.alto - ABAJO - BOTON;
        if (botones..botones + BOTON).contains(&y) {
            return Some(match &self.aceptar {
                Some(_) if x > (interior + HUECO_BOTON) / 2.0 => Pieza::Aceptar,
                _ => Pieza::Cancelar,
            });
        }
        match &self.contenido {
            Contenido::Elegir { mensaje, opciones } => {
                let mut top =
                    ARRIBA + self.alto_titulo() + 6.0 + mensaje.len() as f32 * LH_MENSAJE + 18.0;
                for (n, (_, lineas)) in opciones.iter().enumerate() {
                    let h = alto_opcion(lineas.len());
                    if (top..top + h).contains(&y) {
                        return Some(Pieza::Opcion(n));
                    }
                    top += h + HUECO_OPCION;
                }
                None
            }
            Contenido::Info { grupos, .. } => {
                let mut top = ARRIBA + 76.0 + self.alto_titulo() + 6.0 + LH_MENSAJE;
                for (_, datos) in grupos {
                    top += CABECERA_GRUPO;
                    for d in datos {
                        let h = alto_dato(d);
                        if matches!(d, Dato::Interruptor { .. }) && (top..top + h).contains(&y) {
                            return Some(Pieza::Interruptor);
                        }
                        top += h;
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn accion(&self, pieza: Pieza) -> Option<Accion> {
        match (pieza, &self.contenido) {
            (Pieza::Cancelar, _) => Some(Accion::CerrarEmergente),
            (Pieza::Opcion(n), Contenido::Elegir { opciones, .. }) => {
                opciones.get(n).map(|(o, _)| o.accion.clone())
            }
            (Pieza::Aceptar, Contenido::Renombrar { texto, .. }) => {
                Some(Accion::EscritorioRenombrarA(texto.clone()))
            }
            (Pieza::Interruptor, Contenido::Info { grupos, .. }) => {
                grupos.iter().flat_map(|(_, d)| d).find_map(|d| match d {
                    Dato::Interruptor { accion, .. } => Some(accion.clone()),
                    Dato::Texto(..) => None,
                })
            }
            _ => None,
        }
    }

    /// El interruptor cambia en el sitio al pulsarlo: la acción ya va al
    /// compositor, y esperar a que vuelva para moverlo sería latencia.
    fn alternar(&mut self) {
        if let Contenido::Info { grupos, .. } = &mut self.contenido {
            for d in grupos.iter_mut().flat_map(|(_, d)| d.iter_mut()) {
                if let Dato::Interruptor {
                    encendido, accion, ..
                } = d
                {
                    *encendido = !*encendido;
                    if let Accion::EscritorioEjecutable(v) = accion {
                        *v = !*encendido;
                    }
                }
            }
        }
    }

    // añadido en la 0.2.3
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.pieza_en(x, y));
        let cambia = nueva != self.señalado;
        self.señalado = nueva;
        cambia
    }

    // añadido en la 0.2.3
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let pieza = self.pieza_en(x, y)?;
        let accion = self.accion(pieza);
        if pieza == Pieza::Interruptor {
            self.alternar();
        }
        accion
    }

    // añadido en la 0.2.3
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        if let Contenido::Renombrar {
            texto,
            cursor,
            todo,
            error,
        } = &mut self.contenido
        {
            let letras: Vec<char> = texto.chars().collect();
            let n = letras.len();
            // Primero se decide el texto nuevo y dónde queda el cursor; luego
            // se aplica de una vez.
            let cortar = |desde: usize, hasta: usize, poner: &str| -> (String, usize) {
                let antes: String = letras[..desde].iter().collect();
                let despues: String = letras[hasta..].iter().collect();
                (
                    format!("{antes}{poner}{despues}"),
                    desde + poner.chars().count(),
                )
            };
            let editado = match tecla {
                TeclaPulsada::Caracter(c) if !c.is_control() => Some(if *todo {
                    cortar(0, n, &c.to_string())
                } else {
                    cortar(*cursor, *cursor, &c.to_string())
                }),
                TeclaPulsada::Retroceso if *todo => Some((String::new(), 0)),
                TeclaPulsada::Retroceso if *cursor > 0 => Some(cortar(*cursor - 1, *cursor, "")),
                TeclaPulsada::Retroceso => return Tecla::Consumida,
                TeclaPulsada::BorrarTodo => Some((String::new(), 0)),
                _ => None,
            };
            if let Some((nuevo, c)) = editado {
                *texto = nuevo;
                *cursor = c;
                *todo = false;
                *error = None;
                return Tecla::Consumida;
            }
            let mover = match tecla {
                TeclaPulsada::SeleccionarTodo => {
                    *todo = true;
                    return Tecla::Consumida;
                }
                TeclaPulsada::Izquierda if *todo => Some(0),
                TeclaPulsada::Izquierda => Some(cursor.saturating_sub(1)),
                TeclaPulsada::Derecha if *todo => Some(n),
                TeclaPulsada::Derecha => Some((*cursor + 1).min(n)),
                TeclaPulsada::Inicio => Some(0),
                TeclaPulsada::Fin => Some(n),
                TeclaPulsada::Intro => {
                    return Tecla::Hacer(Accion::EscritorioRenombrarA(texto.clone()));
                }
                _ => None,
            };
            if let Some(c) = mover {
                *cursor = c;
                *todo = false;
                return Tecla::Consumida;
            }
        }
        match tecla {
            TeclaPulsada::Escape => Tecla::Cerrar,
            // Intro en los demás: el principal, o cerrar si no hay.
            TeclaPulsada::Intro => match &self.contenido {
                Contenido::Elegir { opciones, .. } => match self.señalado {
                    Some(Pieza::Opcion(n)) => opciones
                        .get(n)
                        .map_or(Tecla::Consumida, |(o, _)| Tecla::Hacer(o.accion.clone())),
                    _ => Tecla::Consumida,
                },
                _ => Tecla::Cerrar,
            },
            // Arriba y Abajo recorren las opciones de pegar.
            TeclaPulsada::Arriba | TeclaPulsada::Abajo | TeclaPulsada::Tabulador => {
                if let Contenido::Elegir { opciones, .. } = &self.contenido {
                    let n = opciones.len();
                    let actual = match self.señalado {
                        Some(Pieza::Opcion(i)) => Some(i),
                        _ => None,
                    };
                    let siguiente = match (tecla, actual) {
                        (TeclaPulsada::Arriba, None) => n - 1,
                        (TeclaPulsada::Arriba, Some(i)) => (i + n - 1) % n,
                        (_, None) => 0,
                        (_, Some(i)) => (i + 1) % n,
                    };
                    self.señalado = Some(Pieza::Opcion(siguiente));
                }
                Tecla::Consumida
            }
            // Espacio cierra la vista rápida, como la abrió.
            TeclaPulsada::Caracter(' ') if matches!(self.contenido, Contenido::Vista { .. }) => {
                Tecla::Cerrar
            }
            _ => Tecla::Ignorada,
        }
    }

    fn boton(&self, etiqueta: &str, pieza: Pieza, ancho: f32) -> PanelElement<'_> {
        let principal = pieza == Pieza::Aceptar
            || (self.aceptar.is_none() && !matches!(self.contenido, Contenido::Elegir { .. }));
        let base = if principal {
            tema::acento()
        } else {
            tema::superficie_en_tarjeta()
        };
        let fondo = if self.señalado == Some(pieza) {
            tema::mezclar(base, tema::tinta(), 0.12)
        } else {
            base
        };
        let tinta = if principal {
            tema::tinta_sobre(tema::acento())
        } else {
            tema::texto()
        };
        container(
            text(etiqueta.to_string())
                .size(15)
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tinta),
        )
        .center(Length::Fill)
        .width(ancho)
        .height(BOTON)
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

    fn cuerpo(&self) -> PanelElement<'_> {
        let interior = self.interior();
        match &self.contenido {
            Contenido::Elegir { mensaje, opciones } => {
                let mut col = column![Space::new().height(6.0)].align_x(Horizontal::Center);
                for l in mensaje {
                    col = col.push(
                        container(text(l.clone()).size(T_MENSAJE).color(tema::texto2()))
                            .height(LH_MENSAJE)
                            .center_y(LH_MENSAJE),
                    );
                }
                col = col.push(Space::new().height(18.0));
                for (n, (o, lineas)) in opciones.iter().enumerate() {
                    if n > 0 {
                        col = col.push(Space::new().height(HUECO_OPCION));
                    }
                    col = col.push(self.opcion(o, lineas, n, interior));
                }
                col.into()
            }
            Contenido::Renombrar {
                texto,
                cursor,
                todo,
                error,
            } => column![
                Space::new().height(18.0),
                campo(texto, *cursor, *todo, interior),
                container(
                    text(error.clone().unwrap_or_default())
                        .size(T_SUB)
                        .color(super::menu::rojo_texto())
                )
                .height(ERROR)
                .padding(iced_core::Padding {
                    top: 4.0,
                    left: 6.0,
                    ..Default::default()
                }),
            ]
            .into(),
            Contenido::Info {
                subtitulo, grupos, ..
            } => {
                let mut col = column![
                    Space::new().height(6.0),
                    container(
                        text(subtitulo.clone())
                            .size(T_MENSAJE)
                            .color(tema::texto2())
                    )
                    .width(interior)
                    .height(LH_MENSAJE)
                    .center_x(interior)
                ];
                for (titulo, datos) in grupos {
                    col = col.push(
                        container(text(titulo.clone()).size(12).color(tema::texto2()))
                            .height(CABECERA_GRUPO)
                            .padding(iced_core::Padding {
                                top: 18.0,
                                left: 6.0,
                                ..Default::default()
                            }),
                    );
                    col = col.push(self.grupo(datos, interior));
                }
                col.into()
            }
            Contenido::Vista {
                imagen,
                texto,
                icono,
                detalle,
            } => {
                let pieza: PanelElement<'_> = match (imagen, icono) {
                    (Some((h, w, alto)), _) => container(
                        iced_widget::image(h.clone())
                            .width(Length::Fixed(*w))
                            .height(Length::Fixed(*alto)),
                    )
                    .width(interior)
                    .center_x(interior)
                    .into(),
                    (None, _) if !texto.is_empty() => {
                        let mut col = column![];
                        for l in texto {
                            col = col.push(
                                container(
                                    text(l.clone())
                                        .size(12)
                                        .font(Font::MONOSPACE)
                                        .color(tema::texto()),
                                )
                                .height(17.0),
                            );
                        }
                        container(col)
                            .padding(12)
                            .width(interior)
                            .style(|_| container::Style {
                                background: Some(tema::superficie_en_tarjeta().into()),
                                border: Border {
                                    radius: tema::R_CONTROL.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })
                            .into()
                    }
                    (None, Some(ic)) => container(icono::ver(ic, 96.0, 96.0))
                        .width(interior)
                        .center_x(interior)
                        .into(),
                    (None, None) => Space::new().height(96.0).into(),
                };
                column![
                    Space::new().height(6.0),
                    container(text(detalle.clone()).size(T_MENSAJE).color(tema::texto2()))
                        .width(interior)
                        .height(LH_MENSAJE)
                        .center_x(interior),
                    Space::new().height(18.0),
                    pieza,
                ]
                .into()
            }
        }
    }

    fn opcion<'a>(
        &'a self,
        o: &'a Opcion,
        lineas: &'a [String],
        n: usize,
        ancho: f32,
    ) -> PanelElement<'a> {
        let señalada = self.señalado == Some(Pieza::Opcion(n));
        let fondo = if señalada {
            tema::mezclar(tema::superficie_en_tarjeta(), tema::acento(), 0.12)
        } else {
            tema::superficie_en_tarjeta()
        };
        let icono: PanelElement<'_> = match icono::propio(o.icono) {
            Some(ic) => icono::ver_teñido_propio(&ic, 20.0, tema::acento()),
            None => Space::new().into(),
        };
        let mut textos = column![
            text(o.titulo.clone())
                .size(14)
                .font(Font {
                    weight: Weight::Medium,
                    ..Font::DEFAULT
                })
                .color(tema::texto())
        ]
        .spacing(2);
        for l in lineas {
            textos = textos
                .push(container(text(l.clone()).size(T_SUB).color(tema::texto2())).height(LH_SUB));
        }
        container(
            row![
                container(icono)
                    .width(OPCION_ICONO)
                    .height(OPCION_ICONO)
                    .center(OPCION_ICONO)
                    .style(|_| container::Style {
                        background: Some(tema::card().into()),
                        border: Border {
                            radius: tema::R_BOTON_PEQUENO.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                textos,
            ]
            .spacing(12)
            .align_y(Vertical::Center),
        )
        .padding(OPCION_RELLENO)
        .width(ancho)
        .height(alto_opcion(lineas.len()))
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

    fn grupo<'a>(&'a self, datos: &'a [Dato], ancho: f32) -> PanelElement<'a> {
        let mut col = column![];
        for (i, d) in datos.iter().enumerate() {
            let fila: PanelElement<'_> = match d {
                Dato::Texto(etiqueta, valor) => {
                    let sitio = ancho - 32.0 - crate::widget::ancho_de(etiqueta, 14.0) - 14.0;
                    row![
                        text(etiqueta.clone())
                            .size(14)
                            .font(Font {
                                weight: Weight::Medium,
                                ..Font::DEFAULT
                            })
                            .color(tema::texto()),
                        Space::new().width(Length::Fill),
                        text(super::recortar_texto(valor, sitio.max(40.0), 14.0))
                            .size(14)
                            .color(tema::texto2()),
                    ]
                    .align_y(Vertical::Center)
                    .into()
                }
                Dato::Interruptor {
                    titulo,
                    detalle,
                    encendido,
                    ..
                } => {
                    let mut textos = column![
                        text(titulo.clone())
                            .size(14)
                            .font(Font {
                                weight: Weight::Medium,
                                ..Font::DEFAULT
                            })
                            .color(tema::texto())
                    ]
                    .spacing(2);
                    for l in renglones(detalle, ancho - 32.0 - 46.0 - 14.0, T_SUB, 3) {
                        textos = textos.push(text(l).size(T_SUB).color(tema::texto2()));
                    }
                    row![
                        textos.width(Length::Fill),
                        Space::new().width(14.0),
                        super::lista::interruptor(if *encendido { 1.0 } else { 0.0 }),
                    ]
                    .align_y(Vertical::Center)
                    .into()
                }
            };
            col = col.push(
                container(fila)
                    .padding([0, 16])
                    .width(ancho)
                    .height(alto_dato(d))
                    .center_y(alto_dato(d)),
            );
            // Divisor de 0,5 con 16 de sangría, nunca tras la última (HIG §5).
            if i + 1 < datos.len() {
                col = col.push(
                    container(
                        container(Space::new().height(0.5))
                            .width(Length::Fill)
                            .style(|_| container::Style {
                                background: Some(tema::divisor().into()),
                                ..Default::default()
                            }),
                    )
                    .padding(iced_core::Padding {
                        left: 16.0,
                        ..Default::default()
                    }),
                );
            }
        }
        container(col)
            .width(ancho)
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // añadido en la 0.2.3
    pub fn view(&self) -> PanelElement<'_> {
        let interior = self.interior();
        let mut col = column![Space::new().height(ARRIBA)].align_x(Horizontal::Center);
        if let Contenido::Info {
            icono: Some(ic), ..
        } = &self.contenido
        {
            col = col
                .push(icono::ver(ic, 64.0, 64.0))
                .push(Space::new().height(12.0));
        } else if let Contenido::Info { icono: None, .. } = &self.contenido {
            col = col.push(Space::new().height(76.0));
        }
        for l in &self.titulo {
            col = col.push(
                container(
                    text(l.clone())
                        .size(T_TITULO)
                        .font(Font {
                            weight: Weight::Bold,
                            ..Font::DEFAULT
                        })
                        .color(tema::texto()),
                )
                .height(LH_TITULO)
                .center_y(LH_TITULO),
            );
        }
        col = col.push(self.cuerpo());
        col = col.push(Space::new().height(AIRE_BOTONES));
        col = col.push(match &self.aceptar {
            Some(aceptar) => {
                let medio = (interior - HUECO_BOTON) / 2.0;
                PanelElement::from(
                    row![
                        self.boton(&self.cancelar, Pieza::Cancelar, medio),
                        self.boton(aceptar, Pieza::Aceptar, medio)
                    ]
                    .spacing(HUECO_BOTON),
                )
            }
            None => self.boton(&self.cancelar, Pieza::Cancelar, interior),
        });
        let tarjeta = container(container(col).padding([0, LATERAL as u16]))
            .width(self.ancho)
            .height(self.alto)
            .style(|_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            });
        let (ancho, alto) = self.size();
        let franja = SOMBRA + tema::R_DIALOGO;
        let lado = alto - franja * 2.0;
        let [arriba, izquierda, derecha, abajo] = &self.sombra;
        stack![
            column![
                icono::ver(arriba, ancho, franja),
                row![
                    icono::ver(izquierda, SOMBRA, lado),
                    Space::new().width(self.ancho),
                    icono::ver(derecha, SOMBRA, lado),
                ],
                icono::ver(abajo, ancho, franja),
            ],
            container(tarjeta).padding(SOMBRA)
        ]
        .into()
    }
}

/// Pone a cero el alfa fuera de unas esquinas redondeadas de radio `r`
/// píxeles, con un píxel de suavizado. Los colores van sin premultiplicar,
/// como los da el crate `image`.
fn redondear(rgba: &mut [u8], w: u32, h: u32, r: f32) {
    let (w, h) = (w as f32, h as f32);
    for (i, p) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i as f32 % w) + 0.5, (i as f32 / w).floor() + 0.5);
        let dx = (r - x).max(x - (w - r)).max(0.0);
        let dy = (r - y).max(y - (h - r)).max(0.0);
        if dx == 0.0 || dy == 0.0 {
            continue;
        }
        let fuera = (dx.hypot(dy) - r + 0.5).clamp(0.0, 1.0);
        p[3] = (p[3] as f32 * (1.0 - fuera)).round() as u8;
    }
}

fn alto_opcion(lineas: usize) -> f32 {
    (18.0 + 2.0 + lineas as f32 * LH_SUB).max(OPCION_ICONO) + OPCION_RELLENO * 2.0
}

fn alto_dato(d: &Dato) -> f32 {
    match d {
        Dato::Texto(..) => FILA_INFO,
        // Título, hasta tres renglones de explicación y 10 arriba y abajo.
        Dato::Interruptor { detalle, .. } => {
            let n = renglones(
                detalle,
                ANCHO - LATERAL * 2.0 - 32.0 - 46.0 - 14.0,
                T_SUB,
                3,
            )
            .len();
            (20.0 + n as f32 * 15.0 + 20.0).max(FILA_INFO)
        }
    }
}

/// El campo de texto de renombrar: el texto con su cursor, o todo marcado.
fn campo(texto: &str, cursor: usize, todo: bool, ancho: f32) -> PanelElement<'static> {
    let letras: Vec<char> = texto.chars().collect();
    let (antes, despues): (String, String) = (
        letras[..cursor.min(letras.len())].iter().collect(),
        letras[cursor.min(letras.len())..].iter().collect(),
    );
    let tinta = tema::texto();
    let contenido: PanelElement<'static> = if todo && !texto.is_empty() {
        container(text(texto.to_string()).size(14).color(tema::sobre_acento()))
            .padding([1, 2])
            .style(|_| container::Style {
                background: Some(tema::acento().into()),
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    } else {
        row![
            text(antes).size(14).color(tinta),
            container(Space::new().width(1.5).height(18.0)).style(|_| container::Style {
                background: Some(tema::acento().into()),
                ..Default::default()
            }),
            text(despues).size(14).color(tinta),
        ]
        .align_y(Vertical::Center)
        .into()
    };
    container(contenido)
        .padding([0, 14])
        .width(ancho)
        .height(CAMPO)
        .center_y(CAMPO)
        .style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            // Foco siempre: es el único campo del diálogo. Anillo de 2 con el
            // acento, el mínimo del HIG §11.
            border: Border {
                radius: tema::R_BOTON.into(),
                width: 2.0,
                color: tema::acento(),
            },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renombrar_sustituye_y_edita() {
        let mut d = Dialogo::renombrar("informe.pdf");
        d.tecla(TeclaPulsada::Caracter('a'));
        d.tecla(TeclaPulsada::Caracter('b'));
        d.tecla(TeclaPulsada::Izquierda);
        d.tecla(TeclaPulsada::Retroceso);
        assert!(matches!(
            d.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::EscritorioRenombrarA(t)) if t == "b"
        ));
    }

    /// El error no cambia el alto: si cambiara, el compositor rehará la
    /// superficie y el diálogo volvería a entrar.
    #[test]
    fn el_error_no_cambia_el_tamano() {
        let mut d = Dialogo::renombrar("x");
        let antes = d.size();
        d.error("Ya hay algo con ese nombre");
        assert_eq!(d.size(), antes);
    }

    #[test]
    fn pegar_ofrece_acceso_y_copia() {
        let mut d = Dialogo::pegar(&["Proyectos".into()], "Documentos", 3_200_000);
        d.tecla(TeclaPulsada::Abajo);
        assert!(matches!(
            d.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::EscritorioPegarComo(true))
        ));
        d.tecla(TeclaPulsada::Abajo);
        assert!(matches!(
            d.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::EscritorioPegarComo(false))
        ));
    }
}
