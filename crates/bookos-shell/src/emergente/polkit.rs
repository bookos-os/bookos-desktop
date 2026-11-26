//! Diálogo de autenticación de Polkit: contraseña, cambio de usuario y huella.
//!
//! Sigue el patrón visual del diálogo del HIG §9 (como [`super::dialogo`]):
//! tarjeta centrada con velo `.45`, sombra rasterizada una vez al abrir y
//! altura fija calculada en la construcción para no rehacer el búfer ni
//! relanzar la animación de entrada cuando cambia el mensaje de estado, aparece
//! un error o se avisa de Bloq Mayús.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length, Padding};
use iced_widget::{Space, column, container, row, stack, text};

use super::color_carpeta::{SOMBRA, sombra_dialogo};
use super::{Ancla, FilaAccesible, Tecla, t};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const ANCHO: f32 = 380.0;
const LATERAL: f32 = 24.0;
const INTERIOR: f32 = ANCHO - LATERAL * 2.0;
const ARRIBA: f32 = 24.0;
const ABAJO: f32 = 20.0;
const ICONO_H: f32 = 48.0;
const AIRE_TITULO: f32 = 12.0;
const T_TITULO: f32 = 17.0;
const LH_TITULO: f32 = 22.0;
const T_MENSAJE: f32 = 13.0;
const LH_MENSAJE: f32 = 19.0;
const FILA_IDENTIDAD: f32 = 40.0;
const DIVISOR_DETALLE: f32 = 0.5;
const FILA_DETALLE: f32 = 34.0;
const CAMPO: f32 = 44.0;
const OJO_W: f32 = 36.0;
const ESTADO_H: f32 = 34.0;
const LH_ESTADO: f32 = 16.0;
const AIRE_BOTONES: f32 = 10.0;
const BOTON: f32 = 44.0;
const HUECO_BOTON: f32 = 10.0;

/// Una cuenta elegible para satisfacer una solicitud de Polkit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identidad {
    /// Nombre UNIX exacto que se pasa a `polkit-agent-helper-1`.
    pub usuario: String,
    /// Rótulo legible para el diálogo (por ejemplo «Eve (eve)» o «root»).
    pub etiqueta: String,
}

/// Datos iniciales que el agente de Polkit envía para abrir el diálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peticion {
    pub id: u64,
    pub mensaje: String,
    pub icono: String,
    pub detalle: Option<String>,
    pub identidades: Vec<Identidad>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pieza {
    Identidad,
    Ojo,
    Cancelar,
    Autenticar,
}

pub struct Polkit {
    id: u64,
    icono_nombre: String,
    icono: Option<Icono>,
    mensaje: Vec<String>,
    detalle: Option<String>,
    identidades: Vec<Identidad>,
    identidad: usize,
    /// Nunca se registra en trazas; se limpia con escritura volátil en `Drop`.
    clave: String,
    cursor: usize,
    todo: bool,
    oculto: bool,
    mostrar_clave: bool,
    prompt: Option<String>,
    info: Option<String>,
    error: Option<String>,
    comprobando: bool,
    caps_lock: bool,
    alto: f32,
    sombra: [Icono; 4],
    señalado: Option<Pieza>,
}

fn borrar_bytes(bytes: &mut [u8]) {
    for b in bytes {
        // SAFETY: `b` proviene de una referencia mutable viva y válida.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

fn borrar_string(s: &mut String) {
    // SAFETY: rellenar todos los bytes con `0x00` mantiene UTF-8 válido (`\0`).
    unsafe { borrar_bytes(s.as_bytes_mut()) };
    s.clear();
}

fn cargar_icono(nombre: &str) -> Option<Icono> {
    if !nombre.is_empty()
        && let Some(ic) = icono::cargar(nombre)
    {
        return Some(ic);
    }
    icono::insignia("bloquear")
}

impl Drop for Polkit {
    fn drop(&mut self) {
        borrar_string(&mut self.clave);
    }
}

impl Polkit {
    pub fn new(peticion: Peticion) -> Self {
        let mensaje = crate::escritorio::lineas(&peticion.mensaje, INTERIOR, T_MENSAJE, 3);
        let icono = cargar_icono(&peticion.icono);
        let mut dialog = Self {
            id: peticion.id,
            icono_nombre: peticion.icono,
            icono,
            mensaje,
            detalle: peticion.detalle,
            identidades: peticion.identidades,
            identidad: 0,
            clave: String::new(),
            cursor: 0,
            todo: false,
            oculto: true,
            mostrar_clave: false,
            prompt: None,
            info: None,
            error: None,
            comprobando: false,
            caps_lock: false,
            alto: 0.0,
            sombra: sombra_dialogo(ANCHO, 200.0),
            señalado: None,
        };
        dialog.alto = dialog.y_botones() + BOTON + ABAJO;
        dialog.sombra = sombra_dialogo(ANCHO, dialog.alto);
        dialog
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn cancelar(&self) -> Accion {
        Accion::PolkitCancelar { id: self.id }
    }

    /// Extrae los bytes de la contraseña escrita dejando el búfer interno vacío.
    pub fn tomar_clave(&mut self) -> Vec<u8> {
        self.cursor = 0;
        self.todo = false;
        std::mem::take(&mut self.clave).into_bytes()
    }

    pub fn size(&self) -> (f32, f32) {
        (ANCHO + SOMBRA * 2.0, self.alto + SOMBRA * 2.0)
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    pub fn animando(&self) -> bool {
        false
    }

    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    pub fn recargar_iconos(&mut self) {
        self.icono = cargar_icono(&self.icono_nombre);
    }

    pub fn poner_info(&mut self, texto: Option<String>) -> bool {
        if self.info == texto {
            return false;
        }
        self.info = texto;
        true
    }

    pub fn poner_error(&mut self, texto: String) -> bool {
        borrar_string(&mut self.clave);
        self.cursor = 0;
        self.todo = false;
        self.comprobando = false;
        self.error = Some(texto);
        true
    }

    pub fn poner_prompt(&mut self, prompt: Option<String>, oculto: bool) -> bool {
        let cambio = self.prompt != prompt || self.oculto != oculto;
        self.prompt = prompt;
        self.oculto = oculto;
        if !oculto {
            self.mostrar_clave = true;
        }
        cambio
    }

    pub fn poner_caps_lock(&mut self, activo: bool) -> bool {
        if self.caps_lock == activo {
            return false;
        }
        self.caps_lock = activo;
        true
    }

    fn y_identidad(&self) -> f32 {
        ARRIBA
            + ICONO_H
            + AIRE_TITULO
            + LH_TITULO
            + 6.0
            + self.mensaje.len() as f32 * LH_MENSAJE
            + 14.0
    }

    fn alto_contexto(&self) -> f32 {
        FILA_IDENTIDAD
            + if self.detalle.is_some() {
                DIVISOR_DETALLE + FILA_DETALLE
            } else {
                0.0
            }
    }

    fn y_campo(&self) -> f32 {
        self.y_identidad() + self.alto_contexto() + 14.0
    }

    fn y_botones(&self) -> f32 {
        self.y_campo() + CAMPO + 6.0 + ESTADO_H + AIRE_BOTONES
    }

    fn identidad_actual(&self) -> Option<&Identidad> {
        self.identidades.get(self.identidad)
    }

    fn pieza_en(&self, x: f32, y: f32) -> Option<Pieza> {
        let (x, y) = (x - SOMBRA - LATERAL, y - SOMBRA);
        if !(0.0..=INTERIOR).contains(&x) {
            return None;
        }
        let y_id = self.y_identidad();
        if self.identidades.len() > 1 && (y_id..y_id + FILA_IDENTIDAD).contains(&y) {
            return Some(Pieza::Identidad);
        }
        let y_campo = self.y_campo();
        if self.oculto && (y_campo..y_campo + CAMPO).contains(&y) && x >= INTERIOR - OJO_W {
            return Some(Pieza::Ojo);
        }
        let y_bot = self.y_botones();
        if (y_bot..y_bot + BOTON).contains(&y) {
            let medio = (INTERIOR - HUECO_BOTON) / 2.0;
            if x <= medio {
                return Some(Pieza::Cancelar);
            }
            if x >= medio + HUECO_BOTON {
                return Some(Pieza::Autenticar);
            }
        }
        None
    }

    fn reiniciar_al_cambiar_identidad(&mut self) {
        self.identidad = (self.identidad + 1) % self.identidades.len();
        borrar_string(&mut self.clave);
        self.cursor = 0;
        self.todo = false;
        self.oculto = true;
        self.mostrar_clave = false;
        self.prompt = None;
        self.info = None;
        self.error = None;
    }

    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let mut out = Vec::new();
        if self.identidades.len() > 1
            && let Some(id) = self.identidad_actual()
        {
            out.push(FilaAccesible {
                id: "polkit:identidad".into(),
                rol: accesskit::Role::Button,
                nombre: t("Cambiar usuario", "Switch user").into(),
                valor: id.etiqueta.clone(),
                rect: (SOMBRA + LATERAL, SOMBRA + self.y_identidad(), INTERIOR, FILA_IDENTIDAD),
                marcada: None,
                seleccionada: self.señalado == Some(Pieza::Identidad),
            });
        }
        out.push(FilaAccesible {
            id: "polkit:clave".into(),
            rol: accesskit::Role::TextInput,
            nombre: self
                .prompt
                .clone()
                .unwrap_or_else(|| t("Contraseña", "Password").into()),
            valor: String::new(),
            rect: (SOMBRA + LATERAL, SOMBRA + self.y_campo(), INTERIOR, CAMPO),
            marcada: None,
            seleccionada: false,
        });
        let y_bot = SOMBRA + self.y_botones();
        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        out.push(FilaAccesible {
            id: "polkit:cancelar".into(),
            rol: accesskit::Role::Button,
            nombre: t("Cancelar", "Cancel").into(),
            valor: String::new(),
            rect: (SOMBRA + LATERAL, y_bot, medio, BOTON),
            marcada: None,
            seleccionada: self.señalado == Some(Pieza::Cancelar),
        });
        out.push(FilaAccesible {
            id: "polkit:autenticar".into(),
            rol: accesskit::Role::Button,
            nombre: t("Autenticar", "Authenticate").into(),
            valor: String::new(),
            rect: (
                SOMBRA + LATERAL + medio + HUECO_BOTON,
                y_bot,
                medio,
                BOTON,
            ),
            marcada: None,
            seleccionada: self.señalado == Some(Pieza::Autenticar),
        });
        out
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.pieza_en(x, y));
        let cambia = nueva != self.señalado;
        self.señalado = nueva;
        cambia
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let pieza = self.pieza_en(x, y)?;
        match pieza {
            Pieza::Cancelar => Some(self.cancelar()),
            Pieza::Autenticar => {
                if !self.comprobando && !self.clave.is_empty() {
                    self.comprobando = true;
                    self.error = None;
                    Some(Accion::PolkitEnviar { id: self.id })
                } else {
                    None
                }
            }
            Pieza::Ojo => {
                if self.oculto {
                    self.mostrar_clave = !self.mostrar_clave;
                }
                None
            }
            Pieza::Identidad => {
                if !self.comprobando && self.identidades.len() > 1 {
                    self.reiniciar_al_cambiar_identidad();
                    Some(Accion::PolkitCambiarIdentidad {
                        id: self.id,
                        indice: self.identidad,
                    })
                } else {
                    None
                }
            }
        }
    }

    fn reemplazar_texto(&mut self, desde: usize, hasta: usize, insertar: &str) {
        let chars: Vec<char> = self.clave.chars().collect();
        let n = chars.len();
        let d = desde.min(n);
        let h = hasta.min(n).max(d);
        let mut nuevo = String::with_capacity(self.clave.len() + insertar.len());
        for &c in &chars[..d] {
            nuevo.push(c);
        }
        nuevo.push_str(insertar);
        for &c in &chars[h..] {
            nuevo.push(c);
        }
        borrar_string(&mut self.clave);
        self.clave = nuevo;
        self.cursor = d + insertar.chars().count();
        self.todo = false;
        self.error = None;
    }

    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        match tecla {
            TeclaPulsada::Escape => Tecla::Hacer(self.cancelar()),
            _ if self.comprobando => Tecla::Consumida,
            TeclaPulsada::Intro => {
                if !self.clave.is_empty() {
                    self.comprobando = true;
                    self.error = None;
                    Tecla::Hacer(Accion::PolkitEnviar { id: self.id })
                } else {
                    Tecla::Consumida
                }
            }
            TeclaPulsada::Tabulador => {
                if self.identidades.len() > 1 {
                    self.reiniciar_al_cambiar_identidad();
                    Tecla::Hacer(Accion::PolkitCambiarIdentidad {
                        id: self.id,
                        indice: self.identidad,
                    })
                } else {
                    Tecla::Consumida
                }
            }
            TeclaPulsada::Caracter(c) if !c.is_control() => {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                if self.todo {
                    let n = self.clave.chars().count();
                    self.reemplazar_texto(0, n, s);
                } else {
                    self.reemplazar_texto(self.cursor, self.cursor, s);
                }
                borrar_bytes(&mut buf);
                Tecla::Consumida
            }
            TeclaPulsada::Pegar(mut texto) => {
                let limpio: String = texto.chars().filter(|c| !c.is_control()).collect();
                borrar_string(&mut texto);
                if !limpio.is_empty() {
                    let n = self.clave.chars().count();
                    if self.todo {
                        self.reemplazar_texto(0, n, &limpio);
                    } else {
                        self.reemplazar_texto(self.cursor, self.cursor, &limpio);
                    }
                }
                let mut limpio = limpio;
                borrar_string(&mut limpio);
                Tecla::Consumida
            }
            TeclaPulsada::Retroceso => {
                if self.todo {
                    borrar_string(&mut self.clave);
                    self.cursor = 0;
                    self.todo = false;
                    self.error = None;
                } else if self.cursor > 0 {
                    self.reemplazar_texto(self.cursor - 1, self.cursor, "");
                }
                Tecla::Consumida
            }
            TeclaPulsada::Suprimir => {
                let n = self.clave.chars().count();
                if self.todo {
                    borrar_string(&mut self.clave);
                    self.cursor = 0;
                    self.todo = false;
                    self.error = None;
                } else if self.cursor < n {
                    self.reemplazar_texto(self.cursor, self.cursor + 1, "");
                }
                Tecla::Consumida
            }
            TeclaPulsada::BorrarTodo => {
                borrar_string(&mut self.clave);
                self.cursor = 0;
                self.todo = false;
                self.error = None;
                Tecla::Consumida
            }
            TeclaPulsada::SeleccionarTodo => {
                self.todo = !self.clave.is_empty();
                Tecla::Consumida
            }
            TeclaPulsada::Izquierda => {
                self.cursor = if self.todo {
                    0
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.todo = false;
                Tecla::Consumida
            }
            TeclaPulsada::Derecha => {
                let n = self.clave.chars().count();
                self.cursor = if self.todo {
                    n
                } else {
                    (self.cursor + 1).min(n)
                };
                self.todo = false;
                Tecla::Consumida
            }
            TeclaPulsada::Inicio => {
                self.cursor = 0;
                self.todo = false;
                Tecla::Consumida
            }
            TeclaPulsada::Fin => {
                self.cursor = self.clave.chars().count();
                self.todo = false;
                Tecla::Consumida
            }
            _ => Tecla::Consumida,
        }
    }

    fn boton(&self, etiqueta: &'static str, pieza: Pieza, ancho: f32) -> PanelElement<'_> {
        let principal = pieza == Pieza::Autenticar;
        let deshabilitado = principal && (self.clave.is_empty() || self.comprobando);
        let fondo = if principal {
            if deshabilitado {
                tema::alfa(tema::acento(), 0.45)
            } else if self.señalado == Some(pieza) {
                tema::mezclar(tema::acento(), tema::tinta(), 0.12)
            } else {
                tema::acento()
            }
        } else {
            let base = tema::superficie_en_tarjeta();
            if self.señalado == Some(pieza) {
                tema::mezclar(base, tema::tinta(), 0.12)
            } else {
                base
            }
        };
        let tinta = if principal {
            let t = tema::tinta_sobre(tema::acento());
            if deshabilitado {
                tema::alfa(t, 0.72)
            } else {
                t
            }
        } else {
            tema::texto()
        };
        container(
            text(etiqueta)
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

    fn vista_contexto(&self) -> PanelElement<'_> {
        let etiqueta_id = self
            .identidad_actual()
            .map_or_else(String::new, |id| id.etiqueta.clone());
        let usuario_icono: PanelElement<'_> = match icono::propio("usuarios") {
            Some(ic) => icono::ver_teñido_propio(&ic, 16.0, tema::texto2()),
            None => Space::new().width(16.0).into(),
        };
        let cambiar: PanelElement<'_> = if self.identidades.len() > 1 {
            let hover = self.señalado == Some(Pieza::Identidad);
            container(
                text(t("Cambiar", "Switch"))
                    .size(12)
                    .font(Font {
                        weight: Weight::Medium,
                        ..Font::DEFAULT
                    })
                    .color(tema::acento()),
            )
            .padding([3, 8])
            .style(move |_| container::Style {
                background: Some(tema::alfa(tema::acento(), if hover { 0.2 } else { 0.12 }).into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        } else {
            Space::new().into()
        };
        let max_etiqueta = if self.identidades.len() > 1 {
            INTERIOR - 28.0 - 16.0 - 8.0 - 80.0
        } else {
            INTERIOR - 28.0 - 16.0 - 8.0
        };
        let fila_id = container(
            row![
                usuario_icono,
                Space::new().width(8.0),
                text(super::recortar_texto(&etiqueta_id, max_etiqueta, 13.0))
                    .size(13)
                    .font(Font {
                        weight: Weight::Medium,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto()),
                Space::new().width(Length::Fill),
                cambiar,
            ]
            .align_y(Vertical::Center),
        )
        .padding([0, 14])
        .width(INTERIOR)
        .height(FILA_IDENTIDAD)
        .center_y(FILA_IDENTIDAD);

        let mut col = column![fila_id];
        if let Some(detalle) = &self.detalle {
            col = col.push(
                container(Space::new().height(DIVISOR_DETALLE))
                    .width(INTERIOR)
                    .style(|_| container::Style {
                        background: Some(tema::divisor().into()),
                        ..Default::default()
                    }),
            );
            col = col.push(
                container(
                    text(super::recortar_texto(detalle, INTERIOR - 28.0, 11.5))
                        .size(11.5)
                        .font(Font::MONOSPACE)
                        .color(tema::texto2()),
                )
                .padding([0, 14])
                .width(INTERIOR)
                .height(FILA_DETALLE)
                .center_y(FILA_DETALLE),
            );
        }

        container(col)
            .width(INTERIOR)
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

    fn vista_campo(&self) -> PanelElement<'_> {
        let placeholder = self
            .prompt
            .clone()
            .unwrap_or_else(|| t("Contraseña", "Password").into());
        let visible_chars = if self.oculto && !self.mostrar_clave {
            vec!['•'; self.clave.chars().count()]
        } else {
            self.clave.chars().collect()
        };
        let n = visible_chars.len();
        let max_chars: usize = 24;
        let inicio = if n > max_chars {
            (self.cursor.saturating_sub(max_chars - 2)).min(n - max_chars)
        } else {
            0
        };
        let fin = (inicio + max_chars).min(n);
        let cursor_rel = self.cursor.clamp(inicio, fin) - inicio;
        let ventana = &visible_chars[inicio..fin];
        let tinta = tema::texto();
        let texto_el: PanelElement<'_> = if self.clave.is_empty() {
            row![
                container(Space::new().width(1.5).height(18.0)).style(|_| container::Style {
                    background: Some(tema::acento().into()),
                    ..Default::default()
                }),
                Space::new().width(2.0),
                text(placeholder).size(14).color(tema::texto2()),
            ]
            .align_y(Vertical::Center)
            .into()
        } else if self.todo {
            let todo_texto: String = ventana.iter().collect();
            container(text(todo_texto).size(14).color(tema::sobre_acento()))
                .padding([1, 3])
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
            let antes: String = ventana[..cursor_rel].iter().collect();
            let despues: String = ventana[cursor_rel..].iter().collect();
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

        let ojo: PanelElement<'_> = if self.oculto {
            let nombre = if self.mostrar_clave {
                "ojo-tachado"
            } else {
                "ojo"
            };
            let color_ojo = if self.señalado == Some(Pieza::Ojo) {
                tema::texto()
            } else {
                tema::texto2()
            };
            let ic_el: PanelElement<'_> = match icono::propio(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, 16.0, color_ojo),
                None => Space::new().into(),
            };
            container(ic_el).width(OJO_W).height(CAMPO).center(CAMPO).into()
        } else {
            Space::new().width(12.0).into()
        };

        let borde_color = if self.error.is_some() {
            super::menu::rojo_texto()
        } else if self.comprobando {
            tema::borde()
        } else {
            tema::acento()
        };
        let borde_ancho = if self.comprobando || self.error.is_some() {
            1.5
        } else {
            2.0
        };

        container(
            row![
                container(texto_el)
                    .padding(Padding {
                        left: 14.0,
                        right: 4.0,
                        ..Default::default()
                    })
                    .width(Length::Fill)
                    .height(CAMPO)
                    .center_y(CAMPO),
                ojo,
            ]
            .align_y(Vertical::Center),
        )
        .width(INTERIOR)
        .height(CAMPO)
        .style(move |_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                width: borde_ancho,
                color: borde_color,
            },
            ..Default::default()
        })
        .into()
    }

    fn vista_estado(&self) -> PanelElement<'_> {
        let (lineas, color): (Vec<String>, Color) = if let Some(err) = &self.error {
            (
                crate::escritorio::lineas(err, INTERIOR, 11.5, 2),
                super::menu::rojo_texto(),
            )
        } else if self.comprobando {
            (
                vec![t("Comprobando…", "Authenticating…").into()],
                tema::texto2(),
            )
        } else if self.caps_lock {
            (
                vec![t("Bloq Mayús activado", "Caps Lock is on").into()],
                tema::acento(),
            )
        } else if let Some(info) = &self.info {
            (
                crate::escritorio::lineas(info, INTERIOR, 11.5, 2),
                tema::acento(),
            )
        } else {
            (Vec::new(), tema::texto2())
        };

        let mut col = column![].align_x(Horizontal::Center);
        for l in lineas {
            col = col.push(
                container(text(l).size(11.5).color(color))
                    .height(LH_ESTADO)
                    .center_y(LH_ESTADO),
            );
        }
        container(col)
            .width(INTERIOR)
            .height(ESTADO_H)
            .center_x(INTERIOR)
            .center_y(ESTADO_H)
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(ARRIBA)].align_x(Horizontal::Center);
        let cabecera_icono: PanelElement<'_> = match &self.icono {
            Some(ic) => icono::ver(ic, ICONO_H, ICONO_H),
            None => Space::new().width(ICONO_H).height(ICONO_H).into(),
        };
        col = col
            .push(cabecera_icono)
            .push(Space::new().height(AIRE_TITULO))
            .push(
                container(
                    text(t("Autenticación requerida", "Authentication required"))
                        .size(T_TITULO)
                        .font(Font {
                            weight: Weight::Bold,
                            ..Font::DEFAULT
                        })
                        .color(tema::texto()),
                )
                .height(LH_TITULO)
                .center_y(LH_TITULO),
            )
            .push(Space::new().height(6.0));
        for l in &self.mensaje {
            col = col.push(
                container(text(l.clone()).size(T_MENSAJE).color(tema::texto2()))
                    .height(LH_MENSAJE)
                    .center_y(LH_MENSAJE),
            );
        }
        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        col = col
            .push(Space::new().height(14.0))
            .push(self.vista_contexto())
            .push(Space::new().height(14.0))
            .push(self.vista_campo())
            .push(Space::new().height(6.0))
            .push(self.vista_estado())
            .push(Space::new().height(AIRE_BOTONES))
            .push(
                row![
                    self.boton(t("Cancelar", "Cancel"), Pieza::Cancelar, medio),
                    self.boton(t("Autenticar", "Authenticate"), Pieza::Autenticar, medio),
                ]
                .spacing(HUECO_BOTON),
            );

        let tarjeta = container(container(col).padding([0, LATERAL as u16]))
            .width(ANCHO)
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
                    Space::new().width(ANCHO),
                    icono::ver(derecha, SOMBRA, lado),
                ],
                icono::ver(abajo, ancho, franja),
            ],
            container(tarjeta).padding(SOMBRA)
        ]
        .into()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn peticion_ejemplo(dos_identidades: bool) -> Peticion {
        let mut identidades = vec![Identidad {
            usuario: "eve".into(),
            etiqueta: "Eve (eve)".into(),
        }];
        if dos_identidades {
            identidades.push(Identidad {
                usuario: "root".into(),
                etiqueta: "root".into(),
            });
        }
        Peticion {
            id: 42,
            mensaje: "Se requiere autenticación para ejecutar un programa como superusuario.".into(),
            icono: "dialog-password".into(),
            detalle: Some("/usr/bin/true".into()),
            identidades,
        }
    }

    #[test]
    fn escribir_editar_y_enviar_extrae_clave_limpia() {
        let mut p = Polkit::new(peticion_ejemplo(false));
        for c in "secreto".chars() {
            assert!(matches!(p.tecla(TeclaPulsada::Caracter(c)), Tecla::Consumida));
        }
        p.tecla(TeclaPulsada::Izquierda);
        p.tecla(TeclaPulsada::Retroceso);
        p.tecla(TeclaPulsada::Caracter('x'));
        p.tecla(TeclaPulsada::Fin);
        p.tecla(TeclaPulsada::Caracter('!'));

        assert!(matches!(
            p.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::PolkitEnviar { id: 42 })
        ));
        let clave = p.tomar_clave();
        assert_eq!(clave, b"secrexo!");
        assert!(p.clave.is_empty());
    }

    #[test]
    fn ctrl_a_ctrl_u_y_pegar_filtran_controles() {
        let mut p = Polkit::new(peticion_ejemplo(false));
        p.tecla(TeclaPulsada::Pegar("abc\ndef\t1".into()));
        assert_eq!(p.clave, "abcdef1");
        p.tecla(TeclaPulsada::SeleccionarTodo);
        p.tecla(TeclaPulsada::Caracter('z'));
        assert_eq!(p.clave, "z");
        p.tecla(TeclaPulsada::BorrarTodo);
        assert!(p.clave.is_empty());
    }

    #[test]
    fn intro_vacio_no_envia_y_escape_siempre_cancela() {
        let mut p = Polkit::new(peticion_ejemplo(false));
        assert!(matches!(p.tecla(TeclaPulsada::Intro), Tecla::Consumida));

        p.tecla(TeclaPulsada::Caracter('a'));
        assert!(matches!(
            p.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::PolkitEnviar { id: 42 })
        ));
        assert!(p.comprobando);
        // Mientras comprueba no edita, pero Escape sigue cancelando.
        p.tecla(TeclaPulsada::Caracter('b'));
        assert_eq!(p.clave, "a");
        assert!(matches!(
            p.tecla(TeclaPulsada::Escape),
            Tecla::Hacer(Accion::PolkitCancelar { id: 42 })
        ));
    }

    #[test]
    fn raton_ojo_identidad_y_botones_funcionan_en_sus_rectangulos() {
        let mut p = Polkit::new(peticion_ejemplo(true));
        let cx_id = SOMBRA + LATERAL + INTERIOR / 2.0;
        let cy_id = SOMBRA + p.y_identidad() + FILA_IDENTIDAD / 2.0;
        assert!(matches!(
            p.pulsar(cx_id, cy_id),
            Some(Accion::PolkitCambiarIdentidad { id: 42, indice: 1 })
        ));

        let cx_ojo = SOMBRA + LATERAL + INTERIOR - OJO_W / 2.0;
        let cy_ojo = SOMBRA + p.y_campo() + CAMPO / 2.0;
        assert!(!p.mostrar_clave);
        assert!(p.pulsar(cx_ojo, cy_ojo).is_none());
        assert!(p.mostrar_clave);

        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        let cx_cancel = SOMBRA + LATERAL + medio / 2.0;
        let cy_bot = SOMBRA + p.y_botones() + BOTON / 2.0;
        assert!(matches!(
            p.pulsar(cx_cancel, cy_bot),
            Some(Accion::PolkitCancelar { id: 42 })
        ));

        let cx_ok = SOMBRA + LATERAL + medio + HUECO_BOTON + medio / 2.0;
        assert!(p.pulsar(cx_ok, cy_bot).is_none()); // clave vacía
        p.tecla(TeclaPulsada::Caracter('1'));
        assert!(matches!(
            p.pulsar(cx_ok, cy_bot),
            Some(Accion::PolkitEnviar { id: 42 })
        ));
    }

    #[test]
    fn error_limpia_clave_y_conserva_tamano_fijo() {
        let mut p = Polkit::new(peticion_ejemplo(false));
        let tam = p.size();
        p.tecla(TeclaPulsada::Caracter('1'));
        p.tecla(TeclaPulsada::Intro);
        assert!(p.comprobando);
        p.poner_info(Some("Coloca el dedo en el lector".into()));
        assert_eq!(p.size(), tam);
        p.poner_error("Contraseña incorrecta. Inténtalo de nuevo.".into());
        assert!(!p.comprobando);
        assert!(p.clave.is_empty());
        assert_eq!(p.size(), tam);
    }
}
