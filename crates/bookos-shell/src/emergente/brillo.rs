//! El emergente del brillo, colgado de su icono.
//!
//! Pantalla y teclado se detectan de nuevo al refrescar. Las escrituras
//! se agrupan en un hilo dedicado para no interrumpir el renderizado.
//!
//! Con sensor de luz, cada fila lleva un botón «A» que la deja en manos del
//! compositor. La tarjeta solo cambia la preferencia: quien lee el sensor y
//! mueve el brillo es el compositor, que sigue haciéndolo con la tarjeta
//! cerrada.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, stack, text};

use crate::Accion;
use crate::config::BrilloAutomatico;
use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

use super::control::{
    self, BAJO_ETIQUETA, BOTON, ETIQUETA, FILA_PILDORA, GRUPO, HUECO, ICONO, MARGEN_AGARRE, PILDORA,
};
use super::lista::{ANCHO, BAJO_CABECERA, CABECERA, FILA, MARGEN};
use super::{Ancla, Tecla};

/// Cuánto más se meten las píldoras que el grupo de abajo: las deja a 20 del
/// borde, alineadas con el título.
const SANGRIA: f32 = 4.0;
/// Ancho del bloque de las píldoras.
const BLOQUE: f32 = ANCHO - (MARGEN + SANGRIA) * 2.0;
/// Hueco entre la fila de la pantalla y la etiqueta del teclado.
const SEPARACION_FILAS: f32 = 14.0;
/// Lo que ocupa una sección: etiqueta, su aire y la fila de la píldora.
const SECCION: f32 = ETIQUETA + BAJO_ETIQUETA + FILA_PILDORA;

/// Alto de la línea de ayuda bajo una fila en automático: 6 de aire y 15 de
/// texto a 11 px.
const AYUDA: f32 = 21.0;
/// Lo que tarda la píldora en seguir un cambio que no hizo la mano. El
/// automático sube en rampa, pero la tarjeta solo se entera de algunos pasos:
/// sin esto la píldora iría a saltos.
const D_BARRA: std::time::Duration = std::time::Duration::from_millis(440);
/// Desde cuánto ajuste a mano se enseña la marca del sensor. Una diferencia
/// menor apenas separa la marca del extremo de la píldora.
const UMBRAL_AJUSTE: u8 = 4;

/// Cuánto sube o baja una muesca de rueda o una flecha.
const PASO: i32 = 5;

/// Por debajo de esto la pantalla se queda negra y el portátil parece apagado.
///
/// El plasmoide usa el mismo suelo. No es un capricho de diseño: con el brillo
/// a cero no se ve ni el propio deslizador para volver a subirlo.
const MINIMO: u8 = 5;

/// La retroiluminación del teclado: dispositivo en `/sys/class/leds`, valor
/// actual y máximo. Va con niveles enteros —en este portátil, cuatro pasos— y
/// no en tanto por ciento, así que se guarda crudo y se enseña convertido.
#[derive(PartialEq, Eq)]
struct Teclado {
    dispositivo: String,
    nivel: u32,
    maximo: u32,
}

/// Cuál es la luz del teclado, si la hay. Mismo criterio que el compositor al
/// atender la tecla XF86: el primer `led` cuyo nombre lleve `kbd_backlight`.
fn leds_teclado() -> Option<Teclado> {
    let (dispositivo, nivel, maximo) = crate::brillo_teclado_actual()?;
    Some(Teclado {
        dispositivo,
        nivel,
        maximo,
    })
}

/// Qué píldora se está arrastrando. Sin esto, agarrar la del teclado movía la
/// de la pantalla, porque `agarrada` era un booleano suelto.
#[derive(PartialEq, Clone, Copy)]
enum Agarre {
    Nada,
    Pantalla,
    Teclado,
}

pub struct Brillo {
    nivel: u8,
    agarrada: Agarre,
    icono: Option<Icono>,
    /// Qué dispositivo de retroiluminación se maneja: `intel_backlight`,
    /// `amdgpu_bl0`… El nombre hace falta para la llamada a logind.
    dispositivo: Option<String>,
    /// Valor crudo máximo del dispositivo, para pasar de tanto por ciento.
    maximo: u32,
    /// La luz del teclado. `None` en un equipo que no la tenga: entonces la
    /// segunda fila ni se dibuja, en vez de enseñar un control muerto.
    teclado: Option<Teclado>,
    icono_teclado: Option<Icono>,
    /// Sin sensor no hay botón «A»: uno que no puede hacer nada promete lo que
    /// no hay.
    sensor: bool,
    automatico: BrilloAutomatico,
    /// El nivel que pide la luz sin el ajuste a mano, si el compositor lo sabe.
    objetivo: Option<u8>,
    /// De qué nivel viene la píldora y desde cuándo, cuando el cambio no lo hizo
    /// la mano.
    transicion: Option<(u8, std::time::Instant)>,
    icono_auto: Option<Icono>,
    /// El puntero sobre el botón «A» de cada fila: 0 pantalla, 1 teclado.
    boton: tema::Realce,
    /// Si la luz nocturna está tiñendo, y su horario si lo tiene.
    nocturna: bool,
    horario: Option<(tema::HoraDelDia, tema::HoraDelDia)>,
}

#[derive(Clone, Copy)]
enum Fila {
    Pantalla,
    Teclado,
}

impl Brillo {
    pub fn new() -> Self {
        let dispositivo = crate::state::backlight_device();
        let maximo = dispositivo
            .as_deref()
            .and_then(crate::state::backlight_max)
            .unwrap_or(0);
        Self {
            nivel: crate::state::read_brightness().unwrap_or(0),
            agarrada: Agarre::Nada,
            icono: crate::icono::propio("brillo"),
            dispositivo,
            maximo,
            teclado: leds_teclado(),
            icono_teclado: crate::icono::propio("teclado"),
            sensor: crate::retroiluminacion::hay_sensor_luz(),
            automatico: crate::retroiluminacion::automatico(),
            objetivo: crate::retroiluminacion::objetivo_sensor(),
            transicion: None,
            icono_auto: crate::icono::propio("brillo-auto"),
            boton: tema::Realce::nuevo(),
            nocturna: crate::luz_nocturna_encendida(),
            horario: horario_nocturna(),
        }
    }

    pub fn refrescar(&mut self) -> bool {
        if self.agarrado() {
            return false;
        }
        let device = crate::backlight();
        let (dispositivo, maximo) = device.map(|(d, m)| (Some(d), m)).unwrap_or((None, 0));
        let nivel = crate::brillo_actual().unwrap_or(0);
        let teclado = leds_teclado();
        let sensor = crate::retroiluminacion::hay_sensor_luz();
        let automatico = crate::retroiluminacion::automatico();
        let objetivo = crate::retroiluminacion::objetivo_sensor();
        let nocturna = crate::luz_nocturna_encendida();
        if nivel != self.nivel && !tema::efectos_reducidos() {
            self.transicion = Some((self.nivel_visible(), std::time::Instant::now()));
        }
        let changed = self.objetivo != objetivo
            || self.dispositivo != dispositivo
            || self.maximo != maximo
            || self.nivel != nivel
            || self.teclado != teclado
            || self.sensor != sensor
            || self.automatico != automatico
            || self.nocturna != nocturna;
        self.nocturna = nocturna;
        self.dispositivo = dispositivo;
        self.maximo = maximo;
        self.nivel = nivel;
        self.teclado = teclado;
        self.sensor = sensor;
        self.automatico = automatico;
        self.objetivo = objetivo;
        changed
    }

    /// El nivel que dibuja la píldora: el real, o de camino hacia él.
    fn nivel_visible(&self) -> u8 {
        match self.transicion {
            Some((desde, t)) if t.elapsed() < D_BARRA => {
                let avance = tema::C_ENTRADA.eval(tema::fraccion(t.elapsed(), D_BARRA));
                (f32::from(desde) + (f32::from(self.nivel) - f32::from(desde)) * avance).round()
                    as u8
            }
            _ => self.nivel,
        }
    }

    fn pantalla_automatica(&self) -> bool {
        self.sensor && self.automatico.pantalla && self.dispositivo.is_some()
    }

    /// Cuánto se ha movido la pantalla a mano respecto a lo que pide la luz,
    /// si es bastante para enseñarlo.
    fn ajuste(&self) -> Option<(u8, i32)> {
        let objetivo = self.objetivo.filter(|_| self.pantalla_automatica())?;
        let ajuste = i32::from(self.nivel) - i32::from(objetivo);
        (ajuste.unsigned_abs() >= u32::from(UMBRAL_AJUSTE)).then_some((objetivo, ajuste))
    }

    /// La línea gris bajo la pantalla. Corta a propósito: a 11 px caben unos
    /// 45 caracteres en el ancho de la tarjeta, y una segunda línea movería
    /// todo lo de abajo.
    fn ayuda_pantalla(&self) -> Option<String> {
        if !self.pantalla_automatica() {
            return None;
        }
        Some(match self.ajuste() {
            Some((_, ajuste)) => format!(
                "Ajustado {}{} % sobre la luz",
                if ajuste > 0 { "+" } else { "−" },
                ajuste.unsigned_abs()
            ),
            None => "Sigue la luz ambiente".into(),
        })
    }

    fn ayuda_teclado(&self) -> Option<&'static str> {
        (self.sensor && self.automatico.teclado && self.teclado.is_some())
            .then_some("Se enciende con poca luz")
    }

    fn alto_ayuda<T>(ayuda: &Option<T>) -> f32 {
        if ayuda.is_some() { AYUDA } else { 0.0 }
    }

    pub fn size(&self) -> (f32, f32) {
        let teclado = if self.teclado.is_some() {
            SEPARACION_FILAS + SECCION + Self::alto_ayuda(&self.ayuda_teclado())
        } else {
            0.0
        };
        (
            ANCHO,
            MARGEN * 2.0
                + CABECERA
                + BAJO_CABECERA
                + SECCION
                + Self::alto_ayuda(&self.ayuda_pantalla())
                + teclado
                + BAJO_CABECERA
                + GRUPO * 2.0
                + FILA,
        )
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("brillo")
    }

    /// El rectángulo de la píldora, relativo a la emergente.
    fn rect_pildora(&self) -> iced_core::Rectangle {
        iced_core::Rectangle {
            x: MARGEN + SANGRIA + ICONO + HUECO,
            y: MARGEN
                + CABECERA
                + BAJO_CABECERA
                + ETIQUETA
                + BAJO_ETIQUETA
                + (FILA_PILDORA - PILDORA) / 2.0,
            width: control::ancho_pildora(BLOQUE, self.sensor),
            height: PILDORA,
        }
    }

    /// El botón «A» de una fila, a la derecha de su píldora.
    fn rect_boton(&self, fila: Fila) -> Option<iced_core::Rectangle> {
        if !self.sensor {
            return None;
        }
        let p = match fila {
            Fila::Pantalla => self.rect_pildora(),
            Fila::Teclado => self.rect_teclado()?,
        };
        Some(iced_core::Rectangle {
            x: p.x + p.width + HUECO,
            y: p.y - (FILA_PILDORA - PILDORA) / 2.0,
            width: BOTON,
            height: BOTON,
        })
    }

    pub fn animando(&self) -> bool {
        self.boton.animando() || self.transicion.is_some_and(|(_, t)| t.elapsed() < D_BARRA)
    }

    /// El rectángulo de la píldora del teclado, relativo a la emergente.
    fn rect_teclado(&self) -> Option<iced_core::Rectangle> {
        self.teclado.as_ref()?;
        let mut r = self.rect_pildora();
        r.y += SECCION + SEPARACION_FILAS + Self::alto_ayuda(&self.ayuda_pantalla());
        Some(r)
    }

    /// La fila de la luz nocturna: el grupo del pie, entero.
    fn rect_nocturna(&self) -> iced_core::Rectangle {
        let alto = GRUPO * 2.0 + FILA;
        iced_core::Rectangle {
            x: MARGEN,
            y: self.size().1 - MARGEN - alto,
            width: ANCHO - MARGEN * 2.0,
            height: alto,
        }
    }

    /// La línea de debajo de «Luz nocturna»: qué hace ahora y hasta cuándo.
    fn estado_nocturna(&self) -> String {
        let hora = |(h, m): tema::HoraDelDia| format!("{h:02}:{m:02}");
        match (self.nocturna, self.horario) {
            (true, Some((_, hasta))) => format!("Hasta las {}", hora(hasta)),
            (true, None) => "Encendida".into(),
            (false, Some((desde, _))) => format!("Programada desde las {}", hora(desde)),
            (false, None) => "Apagada".into(),
        }
    }

    /// El chip «Config» de la cabecera, pegado a la derecha.
    fn rect_config(&self) -> iced_core::Rectangle {
        let ancho = control::ancho_chip("Config");
        iced_core::Rectangle {
            x: ANCHO - MARGEN - SANGRIA - ancho,
            y: MARGEN + (CABECERA - control::CHIP) / 2.0,
            width: ancho,
            height: control::CHIP,
        }
    }

    pub fn agarrado(&self) -> bool {
        self.agarrada != Agarre::Nada
    }

    /// Manda el nivel crudo del teclado a logind. `true` si cambió.
    ///
    /// El nivel llega en tanto por ciento porque es lo que da la píldora, pero
    /// el dispositivo tiene pasos enteros: aquí son cuatro, así que se redondea
    /// al paso más cercano y la píldora salta de 0 a 33, 66 y 100.
    fn poner_teclado(&mut self, porciento: u8) -> bool {
        let Some(teclado) = self.teclado.as_mut() else {
            return false;
        };
        let crudo = (teclado.maximo as f64 * porciento as f64 / 100.0).round() as u32;
        let crudo = crudo.min(teclado.maximo);
        if crudo == teclado.nivel {
            return false;
        }
        teclado.nivel = crudo;
        crate::retroiluminacion::solicitar("leds", &teclado.dispositivo, crudo);
        true
    }

    /// El nivel del teclado en tanto por ciento, para la píldora.
    fn porciento_teclado(&self) -> u8 {
        self.teclado
            .as_ref()
            .map(|t| (u64::from(t.nivel) * 100 / u64::from(t.maximo.max(1))) as u8)
            .unwrap_or(0)
    }

    /// Escribe el nivel y lo manda a logind. `true` si cambió.
    fn poner(&mut self, nivel: u8) -> bool {
        if self.dispositivo.is_none() || self.maximo == 0 {
            return false;
        }
        let nivel = nivel.clamp(MINIMO, 100);
        if nivel == self.nivel {
            return false;
        }
        // Lo mueve la mano: la píldora va con el dedo, sin perseguirlo.
        self.transicion = None;
        self.nivel = nivel;
        let (Some(dispositivo), true) = (self.dispositivo.as_deref(), self.maximo > 0) else {
            return true;
        };
        // De tanto por ciento al valor crudo del dispositivo, que en este panel
        // llega a 400. Comprobado de punta a punta: pedir el 60 % deja
        // `actual_brightness` en 240, que es justo 400 × 0,6.
        let crudo = (self.maximo as f64 * nivel as f64 / 100.0).round() as u32;
        crate::retroiluminacion::solicitar("backlight", dispositivo, crudo);
        true
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if self.agarrada == Agarre::Nada {
            let sobre = punto.and_then(|(x, y)| {
                let p = iced_core::Point::new(x, y);
                [Fila::Pantalla, Fila::Teclado]
                    .into_iter()
                    .position(|f| self.rect_boton(f).is_some_and(|r| r.contains(p)))
            });
            return self.boton.señalar(sobre);
        }
        let Some((x, _)) = punto else {
            return false;
        };
        let r = self.rect_pildora();
        let nivel = control::nivel_en(x, r.x, r.width);
        match self.agarrada {
            Agarre::Nada => false,
            Agarre::Pantalla => self.poner(nivel),
            Agarre::Teclado => self.poner_teclado(nivel),
        }
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        let holgada = |mut zona: iced_core::Rectangle| {
            zona.y -= MARGEN_AGARRE;
            zona.height += MARGEN_AGARRE * 2.0;
            zona
        };
        if self.rect_config().contains(punto) {
            return Some(Accion::Lanzar("bookos-settings --page pantalla".into()));
        }
        if self.rect_nocturna().contains(punto) {
            // Se da por hecho sin esperar al compositor, como la baldosa.
            self.nocturna = !self.nocturna;
            return Some(Accion::LuzNocturna(self.nocturna));
        }
        for fila in [Fila::Pantalla, Fila::Teclado] {
            if self.rect_boton(fila).is_some_and(|r| r.contains(punto)) {
                match fila {
                    Fila::Pantalla => self.automatico.pantalla ^= true,
                    Fila::Teclado => self.automatico.teclado ^= true,
                }
                return Some(Accion::BrilloAutomatico(self.automatico));
            }
        }
        let r = self.rect_pildora();
        let nivel = control::nivel_en(x, r.x, r.width);
        if holgada(r).contains(punto) {
            self.agarrada = Agarre::Pantalla;
            self.poner(nivel);
        } else if self
            .rect_teclado()
            .is_some_and(|r| holgada(r).contains(punto))
        {
            self.agarrada = Agarre::Teclado;
            self.poner_teclado(nivel);
        }
        None
    }

    pub fn soltar(&mut self) -> bool {
        std::mem::replace(&mut self.agarrada, Agarre::Nada) != Agarre::Nada
    }

    pub fn desplazar(&mut self, _dx: f32, dy: f32) -> bool {
        if dy == 0.0 {
            return false;
        }
        let paso = if dy > 0.0 { PASO } else { -PASO };
        let nivel = (self.nivel as i32 + paso).clamp(0, 100) as u8;
        self.poner(nivel)
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda | T::Abajo => {
                self.poner((self.nivel as i32 - PASO).clamp(0, 100) as u8);
                Tecla::Consumida
            }
            T::Derecha | T::Arriba => {
                self.poner((self.nivel as i32 + PASO).clamp(0, 100) as u8);
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let ancho_pildora = control::ancho_pildora(BLOQUE, self.sensor);
        let boton = |encendido: bool, fila: usize| {
            self.sensor.then(|| {
                control::boton(
                    self.icono_auto.as_ref(),
                    encendido,
                    tema::superficie(),
                    self.boton.intensidad(fila),
                )
            })
        };
        let cabecera = container(
            row![
                control::titulo("Brillo"),
                Space::new().width(Length::Fill),
                control::chip("Config", tema::superficie(), tema::texto()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(CABECERA))
        .padding([0, SANGRIA as u16])
        .center_y(Length::Fixed(CABECERA));

        let valor = if self.dispositivo.is_some() {
            format!("{} %", self.nivel)
        } else {
            "No disponible".into()
        };
        let pildora_pantalla = control::pildora(
            ancho_pildora,
            PILDORA,
            self.nivel_visible(),
            self.dispositivo.is_none(),
            tema::superficie(),
        );
        // La marca de lo que pondría el sensor, solo cuando la mano se ha
        // apartado de ello: sin ajuste coincidiría con el final del relleno.
        let pildora_pantalla: PanelElement<'_> = match self.ajuste() {
            Some((objetivo, _)) => {
                // En el centro del extremo del relleno que tendría a ese nivel.
                let x = (control::lleno(ancho_pildora, PILDORA, objetivo)
                    - PILDORA / 2.0
                    - MARCA / 2.0)
                    .clamp(PILDORA / 2.0, ancho_pildora - PILDORA / 2.0);
                let marca = container(Space::new())
                    .width(Length::Fixed(MARCA))
                    .height(Length::Fixed(PILDORA - 10.0))
                    .style(|_| container::Style {
                        background: Some(tema::alfa(tema::texto(), 0.55).into()),
                        border: Border {
                            radius: (MARCA / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                stack![
                    pildora_pantalla,
                    container(marca).padding(iced_core::Padding::ZERO.left(x).top(5.0)),
                ]
                .into()
            }
            None => pildora_pantalla,
        };
        let mut bloque = column![
            etiqueta("Pantalla", valor, self.pantalla_automatica()),
            Space::new().height(Length::Fixed(BAJO_ETIQUETA)),
            control::fila_pildora(
                self.icono.as_ref(),
                pildora_pantalla,
                boton(self.automatico.pantalla, 0),
            ),
        ];
        if let Some(ayuda) = self.ayuda_pantalla() {
            bloque = bloque.push(linea_ayuda(ayuda));
        }
        if self.teclado.is_some() {
            let porciento = self.porciento_teclado();
            let valor = if porciento == 0 {
                "Apagado".to_string()
            } else {
                format!("{porciento} %")
            };
            bloque = bloque
                .push(Space::new().height(Length::Fixed(SEPARACION_FILAS)))
                .push(etiqueta(
                    "Teclado",
                    valor,
                    self.sensor && self.automatico.teclado,
                ))
                .push(Space::new().height(Length::Fixed(BAJO_ETIQUETA)))
                .push(control::fila_pildora(
                    self.icono_teclado.as_ref(),
                    control::pildora(ancho_pildora, PILDORA, porciento, false, tema::superficie()),
                    boton(self.automatico.teclado, 1),
                ));
            if let Some(ayuda) = self.ayuda_teclado() {
                bloque = bloque.push(linea_ayuda(ayuda.to_string()));
            }
        }

        // Toda la fila es el interruptor; el círculo de la luna dice si está
        // encendida con el mismo lenguaje que el botón «A» de arriba.
        let (fondo_luna, tinta_luna) = if self.nocturna {
            (tema::acento(), tema::sobre_acento())
        } else {
            (tema::superficie(), tema::TEXTO2)
        };
        let luna = container(match crate::icono::propio("noche") {
            Some(ic) => crate::icono::ver_teñido_propio(&ic, 16.0, tinta_luna),
            None => Space::new().width(Length::Fixed(16.0)).into(),
        })
        .width(Length::Fixed(control::BOTON))
        .height(Length::Fixed(control::BOTON))
        .center_x(Length::Fixed(control::BOTON))
        .center_y(Length::Fixed(control::BOTON))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo_luna.into()),
            border: Border {
                radius: (control::BOTON / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let nocturna = container(
            row![
                luna,
                Space::new().width(Length::Fixed(12.0)),
                column![
                    text("Luz nocturna")
                        .size(14.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto()),
                    text(self.estado_nocturna()).size(11.0).color(tema::TEXTO2),
                ],
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(FILA))
        .center_y(Length::Fixed(FILA))
        .padding([0, 12])
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(control::baldosa(0.0).into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(BAJO_CABECERA)),
            container(bloque).padding([0, SANGRIA as u16]),
            Space::new().height(Length::Fixed(BAJO_CABECERA)),
            control::grupo(nocturna.into(), ANCHO - MARGEN * 2.0, GRUPO),
        ];
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// El horario de la luz nocturna, si está en modo programado. Se lee al abrir
/// la tarjeta: cambia desde Ajustes, y entonces la tarjeta no está abierta.
fn horario_nocturna() -> Option<(tema::HoraDelDia, tema::HoraDelDia)> {
    let luz = crate::Config::cargar().luz_nocturna;
    (luz.modo == crate::ModoLuzNocturna::Programada).then_some((luz.desde, luz.hasta))
}

/// Ancho de la marca del sensor sobre la píldora.
const MARCA: f32 = 2.0;

/// La etiqueta de encima de una píldora, con la chapa «AUTO» cuando esa fila
/// va sola. Antes el automático se decía con «Auto · 62 %» en gris, que se
/// leía como parte del número y se perdía.
fn etiqueta<'a>(nombre: &'a str, valor: String, automatico: bool) -> PanelElement<'a> {
    let mut fila = row![
        text(nombre)
            .size(13.0)
            .font(control::peso(Weight::Semibold))
            .color(tema::texto()),
        Space::new().width(Length::Fill),
    ]
    .align_y(Vertical::Center);
    if automatico {
        fila = fila
            .push(
                container(
                    text("AUTO")
                        .size(10.0)
                        .font(control::peso(Weight::Bold))
                        .color(tema::acento()),
                )
                .padding([0, 6])
                .center_y(Length::Fixed(ETIQUETA))
                .style(|_| container::Style {
                    background: Some(tema::alfa(tema::acento(), 0.14).into()),
                    border: Border {
                        radius: tema::R_CHIP.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .push(Space::new().width(Length::Fixed(6.0)));
    }
    container(fila.push(text(valor).size(12.0).color(tema::TEXTO2)))
        .width(Length::Fixed(BLOQUE))
        .height(Length::Fixed(ETIQUETA))
        .center_y(Length::Fixed(ETIQUETA))
        .into()
}

/// La línea gris bajo una fila en automático, alineada con la píldora.
fn linea_ayuda<'a>(ayuda: String) -> PanelElement<'a> {
    container(text(ayuda).size(11.0).color(tema::TEXTO2))
        .padding(
            iced_core::Padding::ZERO
                .left(ICONO + HUECO)
                .top(AYUDA - 15.0),
        )
        .height(Length::Fixed(AYUDA))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El brillo no baja de un suelo: a cero no se ve ni el deslizador con el
    /// que volver a subirlo.
    #[test]
    fn no_se_puede_apagar_del_todo() {
        let mut b = Brillo::new();
        b.nivel = 50;
        b.poner(0);
        assert_eq!(b.nivel, MINIMO);
    }

    /// Y no se pasa de cien por arriba.
    #[test]
    fn no_se_pasa_de_cien() {
        let mut b = Brillo::new();
        b.nivel = 50;
        b.poner(200);
        assert_eq!(b.nivel, 100);
    }

    fn automatica(nivel: u8, objetivo: Option<u8>) -> Brillo {
        let mut b = Brillo::new();
        b.sensor = true;
        b.dispositivo = Some("prueba".into());
        b.automatico.pantalla = true;
        b.nivel = nivel;
        b.objetivo = objetivo;
        b
    }

    /// La marca y el «Ajustado» solo salen cuando la mano se ha apartado de lo
    /// que pide la luz más de lo que el automático ignora.
    #[test]
    fn el_ajuste_a_mano_se_enseña_desde_el_umbral() {
        assert_eq!(automatica(52, Some(52)).ajuste(), None);
        assert_eq!(
            automatica(55, Some(52)).ajuste(),
            None,
            "3 puntos no cuentan"
        );
        assert_eq!(automatica(68, Some(52)).ajuste(), Some((52, 16)));
        assert_eq!(
            automatica(40, Some(52)).ayuda_pantalla().as_deref(),
            Some("Ajustado −12 % sobre la luz")
        );
        assert_eq!(
            automatica(68, None).ayuda_pantalla().as_deref(),
            Some("Sigue la luz ambiente"),
            "sin lectura del sensor no hay con qué comparar"
        );
        let mut sin_sensor = automatica(68, Some(52));
        sin_sensor.sensor = false;
        assert_eq!(sin_sensor.ayuda_pantalla(), None);
    }

    /// La ayuda tiene que caber en una línea: `AYUDA` reserva el alto de una
    /// sola, y una segunda se saldría por encima de la fila del teclado.
    #[test]
    fn la_ayuda_cabe_en_una_linea() {
        let ancho = BLOQUE - ICONO - HUECO;
        for texto in [
            "Sigue la luz ambiente",
            "Ajustado −60 % sobre la luz",
            "Se enciende con poca luz",
        ] {
            let medido = crate::widget::ancho_de(texto, 11.0);
            assert!(medido < ancho, "«{texto}» mide {medido} y caben {ancho}");
        }
    }

    #[test]
    fn soltar_sin_agarrar_no_hace_nada() {
        let mut b = Brillo::new();
        assert!(!b.soltar());
    }
}
