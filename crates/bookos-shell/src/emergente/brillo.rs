//! El emergente del brillo, colgado de su icono.
//!
//! Pantalla y teclado se detectan de nuevo al refrescar. Las escrituras
//! se agrupan en un hilo dedicado para no interrumpir el renderizado.
//!
//! Cada barra es una tarjeta con título, como Pantalla y Sonido del centro de
//! control, y es **la misma barra** (`control::barra`): más gruesa en reposo y
//! gordita mientras se toca. Con sensor de luz, cada tarjeta lleva un
//! interruptor «Auto» que la deja en manos del compositor. La tarjeta solo
//! cambia la preferencia: quien lee el sensor y mueve el brillo es el
//! compositor, que sigue haciéndolo con la tarjeta cerrada.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Length, Padding, Point, Rectangle};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::config::BrilloAutomatico;
use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

use super::control::{self, ENTRE_BARRA, FILA_BARRA, ICONO_BARRA, PISTA};
use super::lista::{self, ANCHO, BAJO_CABECERA, CABECERA, MARGEN};
use super::{Ancla, Tecla};

/// Cuánto más se mete la cabecera que las tarjetas: deja el título a 20 del
/// borde, alineado con el texto de dentro de las tarjetas.
pub(super) const SANGRIA: f32 = 4.0;
/// Ancho de una tarjeta: el de las del centro de control.
pub(super) const ANCHO_TARJETA: f32 = ANCHO - MARGEN * 2.0;
/// Relleno de una tarjeta.
pub(super) const PAD_H: f32 = 16.0;
pub(super) const PAD_ARRIBA: f32 = 14.0;
pub(super) const PAD_ABAJO: f32 = 12.0;
/// La fila del título, con «Auto» y el valor.
pub(super) const TITULO: f32 = 26.0;
/// Aire entre esa fila y la barra.
pub(super) const ENTRE: f32 = 8.0;
/// Hueco entre tarjetas.
pub(super) const ENTRE_TARJETAS: f32 = 10.0;
/// Alto de la línea de ayuda bajo la barra, cuando la hay.
const AYUDA: f32 = 26.0;
/// La fila de la luz nocturna.
const NOCTURNA: f32 = 60.0;
/// Lo que mide la barra: la tarjeta menos su relleno y los dos iconos.
pub(super) const ANCHO_BARRA: f32 = ANCHO_TARJETA - PAD_H * 2.0 - (ICONO_BARRA + ENTRE_BARRA) * 2.0;
/// El valor a la derecha del título, y el interruptor «Auto» a su izquierda.
pub(super) const VALOR: f32 = 52.0;
const AUTO: f32 = 70.0;
/// «Restablecer», al final de la línea de ayuda.
const RESTABLECER: f32 = 84.0;
/// El botón de ajustes de la cabecera.
pub(super) const AJUSTES: f32 = 32.0;

/// Lo que tarda la barra en seguir un cambio que no hizo la mano. El
/// automático sube en rampa, pero la tarjeta solo se entera de algunos pasos:
/// sin esto la barra iría a saltos.
const D_BARRA: std::time::Duration = std::time::Duration::from_millis(440);
/// Desde cuánto ajuste a mano se enseña la marca del sensor. Una diferencia
/// menor apenas la separa del final del relleno.
const UMBRAL_AJUSTE: u8 = 4;

/// Cuánto sube o baja una muesca de rueda o una flecha.
const PASO: i32 = 5;

/// Por debajo de esto la pantalla se queda negra y el portátil parece apagado.
///
/// El plasmoide usa el mismo suelo. No es un capricho de diseño: con el brillo
/// a cero no se ve ni la propia barra para volver a subirlo.
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
// 0.1.0
fn leds_teclado() -> Option<Teclado> {
    let (dispositivo, nivel, maximo) = crate::brillo_teclado_actual()?;
    Some(Teclado {
        dispositivo,
        nivel,
        maximo,
    })
}

/// Qué barra se está arrastrando. Sin esto, agarrar la del teclado movía la
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
    /// La luz del teclado. `None` en un equipo que no la tenga: entonces su
    /// tarjeta ni se dibuja, en vez de enseñar un control muerto.
    teclado: Option<Teclado>,
    icono_teclado: Option<Icono>,
    /// Sin sensor no hay «Auto»: un interruptor que no puede hacer nada
    /// promete lo que no hay.
    sensor: bool,
    automatico: BrilloAutomatico,
    /// El nivel que pide la luz sin el ajuste a mano, si el compositor lo sabe.
    objetivo: Option<u8>,
    /// De qué nivel viene la barra y desde cuándo, cuando el cambio no lo hizo
    /// la mano.
    transicion: Option<(u8, std::time::Instant)>,
    icono_ajustes: Option<Icono>,
    icono_noche: Option<Icono>,
    /// El puntero encima de lo pulsable: 0 y 1 los «Auto», 2 los ajustes,
    /// 3 «Restablecer» y 4 la luz nocturna.
    boton: tema::Realce,
    /// Cuánto engorda la barra agarrada, con muelle: confirma el agarre.
    grosor: tema::Transicion,
    /// El recorrido de los interruptores: «Auto» de la pantalla, el del
    /// teclado y la luz nocturna.
    interruptores: [tema::Transicion; 3],
    /// Si la luz nocturna está tiñendo, y su horario si lo tiene.
    nocturna: bool,
    horario: Option<(tema::HoraDelDia, tema::HoraDelDia)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Fila {
    Pantalla,
    Teclado,
}

impl Brillo {
    // 0.1.0
    // modified 0.2.1 -- tarjetas con la barra común, «Auto» e interruptores
    pub fn new() -> Self {
        let dispositivo = crate::state::backlight_device();
        let maximo = dispositivo
            .as_deref()
            .and_then(crate::state::backlight_max)
            .unwrap_or(0);
        let automatico = crate::retroiluminacion::automatico();
        let nocturna = crate::luz_nocturna_encendida();
        let interruptor = |v: bool| tema::Transicion::nueva(f32::from(u8::from(v)), tema::M_RAPIDO);
        Self {
            nivel: crate::state::read_brightness().unwrap_or(0),
            agarrada: Agarre::Nada,
            icono: crate::icono::propio("brillo"),
            dispositivo,
            maximo,
            teclado: leds_teclado(),
            icono_teclado: crate::icono::propio("teclado"),
            sensor: crate::retroiluminacion::hay_sensor_luz(),
            interruptores: [
                interruptor(automatico.pantalla),
                interruptor(automatico.teclado),
                interruptor(nocturna),
            ],
            automatico,
            objetivo: crate::retroiluminacion::objetivo_sensor(),
            transicion: None,
            icono_ajustes: crate::icono::propio("preferencias"),
            icono_noche: crate::icono::propio("noche"),
            boton: tema::Realce::nuevo(),
            grosor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            nocturna,
            horario: horario_nocturna(),
        }
    }

    // 0.1.0
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
        self.sincronizar_interruptores();
        changed
    }

    /// Lleva los tres interruptores a lo que dice el estado.
    // añadido en la 0.2.1
    fn sincronizar_interruptores(&mut self) {
        let valores = [
            self.automatico.pantalla,
            self.automatico.teclado,
            self.nocturna,
        ];
        for (t, v) in self.interruptores.iter_mut().zip(valores) {
            t.ir_a(f32::from(u8::from(v)));
        }
    }

    /// El nivel que dibuja la barra: el real, o de camino hacia él.
    // 0.1.0
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

    // 0.1.0
    fn pantalla_automatica(&self) -> bool {
        self.sensor && self.automatico.pantalla && self.dispositivo.is_some()
    }

    /// Cuánto se ha movido la pantalla a mano respecto a lo que pide la luz,
    /// si es bastante para enseñarlo.
    // 0.1.0
    fn ajuste(&self) -> Option<(u8, i32)> {
        let objetivo = self.objetivo.filter(|_| self.pantalla_automatica())?;
        let ajuste = i32::from(self.nivel) - i32::from(objetivo);
        (ajuste.unsigned_abs() >= u32::from(UMBRAL_AJUSTE)).then_some((objetivo, ajuste))
    }

    /// La línea gris bajo la pantalla. Corta a propósito: comparte fila con
    /// «Restablecer» y una segunda línea movería todo lo de abajo.
    // 0.1.0
    // modified 0.2.1 -- avisa del mínimo, que es lo que explica por qué no baja
    fn ayuda_pantalla(&self) -> Option<String> {
        if self.dispositivo.is_some() && self.nivel <= MINIMO {
            return Some("Es el mínimo: más abajo no se vería.".into());
        }
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

    // 0.1.0
    fn ayuda_teclado(&self) -> Option<&'static str> {
        (self.sensor && self.automatico.teclado && self.teclado.is_some())
            .then_some("Se enciende con poca luz")
    }

    /// Lo que mide una tarjeta con barra, con o sin línea de ayuda.
    // añadido en la 0.2.1
    fn alto_tarjeta(ayuda: bool) -> f32 {
        PAD_ARRIBA + TITULO + ENTRE + FILA_BARRA + if ayuda { AYUDA } else { 0.0 } + PAD_ABAJO
    }

    fn alto_pantalla(&self) -> f32 {
        Self::alto_tarjeta(self.ayuda_pantalla().is_some())
    }

    fn alto_teclado(&self) -> f32 {
        Self::alto_tarjeta(self.ayuda_teclado().is_some())
    }

    // 0.1.0
    // modified 0.2.1 -- tarjetas con título en vez de píldoras sueltas
    pub fn size(&self) -> (f32, f32) {
        let teclado = if self.teclado.is_some() {
            self.alto_teclado() + ENTRE_TARJETAS
        } else {
            0.0
        };
        (
            ANCHO,
            MARGEN * 2.0
                + CABECERA
                + BAJO_CABECERA
                + self.alto_pantalla()
                + ENTRE_TARJETAS
                + teclado
                + NOCTURNA,
        )
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("brillo")
    }

    /// Dónde empieza la tarjeta de una fila, relativo a la emergente.
    // añadido en la 0.2.1
    fn y_tarjeta(&self, fila: Fila) -> Option<f32> {
        let y = MARGEN + CABECERA + BAJO_CABECERA;
        match fila {
            Fila::Pantalla => Some(y),
            Fila::Teclado => {
                self.teclado.as_ref()?;
                Some(y + self.alto_pantalla() + ENTRE_TARJETAS)
            }
        }
    }

    /// El rectángulo de la barra de una fila, relativo a la emergente. Es la
    /// barra de verdad, de [`PISTA`] de alto: de ahí sale el nivel al arrastrar.
    // 0.1.0
    // modified 0.2.1 -- una por tarjeta
    fn rect_barra(&self, fila: Fila) -> Option<Rectangle> {
        let y = self.y_tarjeta(fila)? + PAD_ARRIBA + TITULO + ENTRE + (FILA_BARRA - PISTA) / 2.0;
        Some(Rectangle {
            x: MARGEN + PAD_H + ICONO_BARRA + ENTRE_BARRA,
            y,
            width: ANCHO_BARRA,
            height: PISTA,
        })
    }

    /// Lo que se puede agarrar de la barra: toda la fila, no solo los 10 px
    /// del surco. Apuntarle a una línea fina con el ratón en movimiento falla,
    /// y fallar aquí cierra la emergente en vez de mover la barra.
    // añadido en la 0.2.1
    fn zona_barra(&self, fila: Fila) -> Option<Rectangle> {
        let mut r = self.rect_barra(fila)?;
        r.y -= (FILA_BARRA - PISTA) / 2.0;
        r.height = FILA_BARRA;
        Some(r)
    }

    /// El interruptor «Auto» de una fila, en la línea del título.
    // 0.1.0
    // modified 0.2.1 -- de un botón «A» redondo a un interruptor con nombre
    fn rect_auto(&self, fila: Fila) -> Option<Rectangle> {
        if !self.sensor {
            return None;
        }
        Some(Rectangle {
            x: MARGEN + ANCHO_TARJETA - PAD_H - VALOR - ENTRE - AUTO,
            y: self.y_tarjeta(fila)? + PAD_ARRIBA,
            width: AUTO,
            height: TITULO,
        })
    }

    /// «Restablecer», al final de la línea de ayuda de la pantalla: vuelve a lo
    /// que pide la luz. Solo cuando hay un ajuste que deshacer.
    // añadido en la 0.2.1
    fn rect_restablecer(&self) -> Option<Rectangle> {
        self.ajuste()?;
        Some(Rectangle {
            x: MARGEN + ANCHO_TARJETA - PAD_H - RESTABLECER,
            y: self.y_tarjeta(Fila::Pantalla)? + PAD_ARRIBA + TITULO + ENTRE + FILA_BARRA,
            width: RESTABLECER,
            height: AYUDA,
        })
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.boton.animando()
            || self.grosor.animando()
            || self.interruptores.iter().any(|t| t.animando())
            || self.transicion.is_some_and(|(_, t)| t.elapsed() < D_BARRA)
    }

    /// La fila de la luz nocturna: la tarjeta entera.
    // 0.1.0
    fn rect_nocturna(&self) -> Rectangle {
        Rectangle {
            x: MARGEN,
            y: self.size().1 - MARGEN - NOCTURNA,
            width: ANCHO_TARJETA,
            height: NOCTURNA,
        }
    }

    /// La línea de debajo de «Luz nocturna»: qué hace ahora y hasta cuándo.
    // 0.1.0
    fn estado_nocturna(&self) -> String {
        let hora = |(h, m): tema::HoraDelDia| format!("{h:02}:{m:02}");
        match (self.nocturna, self.horario) {
            (true, Some((_, hasta))) => format!("Hasta las {}", hora(hasta)),
            (true, None) => "Encendida".into(),
            (false, Some((desde, _))) => format!("Programada desde las {}", hora(desde)),
            (false, None) => "Apagada".into(),
        }
    }

    /// El botón de ajustes de la cabecera, pegado a la derecha.
    // 0.1.0
    // modified 0.2.1 -- de un chip «Config» a un botón redondo con su icono
    fn rect_config(&self) -> Rectangle {
        Rectangle {
            x: ANCHO - MARGEN - SANGRIA - AJUSTES,
            y: MARGEN + (CABECERA - AJUSTES) / 2.0,
            width: AJUSTES,
            height: AJUSTES,
        }
    }

    // 0.1.0
    pub fn agarrado(&self) -> bool {
        self.agarrada != Agarre::Nada
    }

    /// Manda el nivel crudo del teclado a logind. `true` si cambió.
    ///
    /// El nivel llega en tanto por ciento porque es lo que da la barra, pero
    /// el dispositivo tiene pasos enteros: aquí son cuatro, así que se redondea
    /// al paso más cercano y la barra salta de 0 a 33, 66 y 100.
    // 0.1.0
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

    /// El nivel del teclado en tanto por ciento, para la barra.
    // 0.1.0
    fn porciento_teclado(&self) -> u8 {
        self.teclado
            .as_ref()
            .map(|t| (u64::from(t.nivel) * 100 / u64::from(t.maximo.max(1))) as u8)
            .unwrap_or(0)
    }

    /// Escribe el nivel y lo manda a logind. `true` si cambió.
    // 0.1.0
    fn poner(&mut self, nivel: u8) -> bool {
        if self.dispositivo.is_none() || self.maximo == 0 {
            return false;
        }
        let nivel = nivel.clamp(MINIMO, 100);
        if nivel == self.nivel {
            return false;
        }
        // Lo mueve la mano: la barra va con el dedo, sin perseguirlo.
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

    /// Qué hay bajo el puntero, para el realce: 0 y 1 los «Auto», 2 los
    /// ajustes, 3 «Restablecer», 4 la luz nocturna.
    // añadido en la 0.2.1
    fn pulsable_en(&self, p: Point) -> Option<usize> {
        [
            self.rect_auto(Fila::Pantalla),
            self.rect_auto(Fila::Teclado),
            Some(self.rect_config()),
            self.rect_restablecer(),
            Some(self.rect_nocturna()),
        ]
        .into_iter()
        .position(|r| r.is_some_and(|r| r.contains(p)))
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if self.agarrada == Agarre::Nada {
            let sobre = punto.and_then(|(x, y)| self.pulsable_en(Point::new(x, y)));
            return self.boton.señalar(sobre);
        }
        let Some((x, _)) = punto else {
            return false;
        };
        let fila = match self.agarrada {
            Agarre::Teclado => Fila::Teclado,
            _ => Fila::Pantalla,
        };
        let Some(r) = self.rect_barra(fila) else {
            return false;
        };
        let nivel = control::nivel_barra(x, r.x, r.width);
        match self.agarrada {
            Agarre::Nada => false,
            Agarre::Pantalla => self.poner(nivel),
            Agarre::Teclado => self.poner_teclado(nivel),
        }
    }

    // añadido en la 0.2.0 -- la holgura coincide con la zona que acepta la pulsación del deslizador.
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        [Fila::Pantalla, Fila::Teclado]
            .into_iter()
            .filter_map(|f| self.zona_barra(f))
            .any(|zona| zona.contains(Point::new(x, y)))
    }

    // 0.1.0
    // modified 0.2.1 -- «Auto», «Restablecer» y las barras nuevas
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = Point::new(x, y);
        if self.rect_config().contains(punto) {
            return Some(Accion::Lanzar("bookos-settings --page pantalla".into()));
        }
        if self.rect_nocturna().contains(punto) {
            // Se da por hecho sin esperar al compositor, como la baldosa.
            self.nocturna = !self.nocturna;
            self.sincronizar_interruptores();
            return Some(Accion::LuzNocturna(self.nocturna));
        }
        for fila in [Fila::Pantalla, Fila::Teclado] {
            if self.rect_auto(fila).is_some_and(|r| r.contains(punto)) {
                match fila {
                    Fila::Pantalla => self.automatico.pantalla ^= true,
                    Fila::Teclado => self.automatico.teclado ^= true,
                }
                self.sincronizar_interruptores();
                return Some(Accion::BrilloAutomatico(self.automatico));
            }
        }
        if self.rect_restablecer().is_some_and(|r| r.contains(punto))
            && let Some((objetivo, _)) = self.ajuste()
        {
            self.poner(objetivo);
            return None;
        }
        for (fila, agarre) in [
            (Fila::Pantalla, Agarre::Pantalla),
            (Fila::Teclado, Agarre::Teclado),
        ] {
            let (Some(zona), Some(r)) = (self.zona_barra(fila), self.rect_barra(fila)) else {
                continue;
            };
            if zona.contains(punto) {
                self.agarrada = agarre;
                self.grosor.ir_a(1.0);
                let nivel = control::nivel_barra(x, r.x, r.width);
                match agarre {
                    Agarre::Teclado => self.poner_teclado(nivel),
                    _ => self.poner(nivel),
                };
                break;
            }
        }
        None
    }

    // 0.1.0
    pub fn soltar(&mut self) -> bool {
        self.grosor.ir_a(0.0);
        std::mem::replace(&mut self.agarrada, Agarre::Nada) != Agarre::Nada
    }

    // 0.1.0
    pub fn desplazar(&mut self, _dx: f32, dy: f32) -> bool {
        if dy == 0.0 {
            return false;
        }
        let paso = if dy > 0.0 { PASO } else { -PASO };
        let nivel = (self.nivel as i32 + paso).clamp(0, 100) as u8;
        self.poner(nivel)
    }

    // 0.1.0
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

    /// El valor de la derecha del título del teclado: una palabra si son los
    /// cuatro niveles de siempre, que un porcentaje no orienta, y si no, el
    /// porcentaje.
    // añadido en la 0.2.1
    fn valor_teclado(&self) -> String {
        match self.teclado.as_ref() {
            Some(t) if t.maximo == 3 => {
                ["Apagada", "Baja", "Media", "Alta"][t.nivel.min(3) as usize].into()
            }
            _ => match self.porciento_teclado() {
                0 => "Apagada".into(),
                p => format!("{p} %"),
            },
        }
    }

    /// Una tarjeta: título, «Auto» y valor, la barra con sus iconos y, debajo,
    /// la línea de ayuda si la hay.
    // añadido en la 0.2.1
    #[allow(clippy::too_many_arguments)]
    fn tarjeta<'a>(
        &'a self,
        titulo: &'a str,
        valor: String,
        ayuda: Option<String>,
        auto: Option<(f32, f32)>,
        barra: PanelElement<'a>,
        iconos: (&'a Option<Icono>, f32, f32),
        restablecer: bool,
    ) -> PanelElement<'a> {
        let mut cabecera = row![
            text(titulo)
                .size(14.0)
                .font(control::peso(Weight::Medium))
                .color(tema::texto()),
            Space::new().width(Length::Fill),
        ]
        .align_y(Vertical::Center);
        if let Some((encendido, realce)) = auto {
            cabecera = cabecera.push(auto_interruptor(encendido, realce));
        }
        cabecera = cabecera
            .push(Space::new().width(Length::Fixed(ENTRE)))
            .push(
                container(text(valor).size(12.0).color(tema::TEXTO2))
                    .width(Length::Fixed(VALOR))
                    .align_right(Length::Fixed(VALOR)),
            );
        let (icono, pequeño, grande) = iconos;
        let caja = |px: f32| -> PanelElement<'a> {
            let dentro: PanelElement<'a> = match icono {
                Some(ic) => crate::icono::ver_teñido(ic, px, Some(tema::texto())),
                None => Space::new().into(),
            };
            container(dentro)
                .width(Length::Fixed(ICONO_BARRA))
                .height(Length::Fixed(FILA_BARRA))
                .center_x(Length::Fixed(ICONO_BARRA))
                .center_y(Length::Fixed(FILA_BARRA))
                .into()
        };
        let mut cuerpo = column![
            container(cabecera)
                .height(Length::Fixed(TITULO))
                .center_y(Length::Fixed(TITULO)),
            Space::new().height(Length::Fixed(ENTRE)),
            row![
                caja(pequeño),
                Space::new().width(Length::Fixed(ENTRE_BARRA)),
                barra,
                Space::new().width(Length::Fixed(ENTRE_BARRA)),
                caja(grande),
            ]
            .align_y(Vertical::Center)
            .height(Length::Fixed(FILA_BARRA)),
        ];
        let alto = Self::alto_tarjeta(ayuda.is_some());
        if let Some(ayuda) = ayuda {
            let mut linea = row![
                text(ayuda).size(12.0).color(tema::TEXTO2),
                Space::new().width(Length::Fill)
            ]
            .align_y(Vertical::Center);
            if restablecer {
                let realce = self.boton.intensidad(3);
                linea = linea.push(
                    container(
                        text("Restablecer")
                            .size(12.0)
                            .font(control::peso(Weight::Semibold))
                            .color(tema::mezclar(tema::acento(), tema::texto(), realce * 0.35)),
                    )
                    .width(Length::Fixed(RESTABLECER))
                    .align_right(Length::Fixed(RESTABLECER)),
                );
            }
            cuerpo = cuerpo.push(
                container(linea)
                    .height(Length::Fixed(AYUDA))
                    .center_y(Length::Fixed(AYUDA)),
            );
        }
        container(cuerpo)
            .width(Length::Fixed(ANCHO_TARJETA))
            .height(Length::Fixed(alto))
            .padding(Padding {
                top: PAD_ARRIBA,
                bottom: PAD_ABAJO,
                left: PAD_H,
                right: PAD_H,
            })
            .style(|_: &iced_widget::Theme| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.0
    // modified 0.2.1 -- tarjetas con título, «Auto» como interruptor y la luz nocturna como fila
    pub fn view(&self) -> PanelElement<'_> {
        let grueso = |agarre: Agarre| {
            if self.agarrada == agarre {
                control::grueso_barra(self.grosor.valor())
            } else {
                PISTA
            }
        };
        let barra_pantalla = control::barra(
            ANCHO_BARRA,
            grueso(Agarre::Pantalla),
            f32::from(self.nivel_visible()) / 100.0,
            if self.dispositivo.is_none() {
                tema::TEXTO2
            } else {
                tema::acento()
            },
            tema::elevado(),
            &[],
            // La marca de lo que pondría el sensor, solo cuando la mano se ha
            // apartado de ello: sin ajuste coincidiría con el final del relleno.
            self.ajuste()
                .map(|(objetivo, _)| f32::from(objetivo) / 100.0),
        );
        let valor = if self.dispositivo.is_some() {
            format!("{} %", self.nivel)
        } else {
            "—".into()
        };
        let auto = |encendido_idx: usize, realce_idx: usize| {
            self.sensor.then(|| {
                (
                    self.interruptores[encendido_idx].valor(),
                    self.boton.intensidad(realce_idx),
                )
            })
        };
        let mut tarjetas = column![self.tarjeta(
            "Pantalla",
            valor,
            self.ayuda_pantalla(),
            auto(0, 0),
            barra_pantalla,
            (&self.icono, 17.0, 24.0),
            self.rect_restablecer().is_some(),
        )]
        .spacing(ENTRE_TARJETAS);
        if let Some(t) = self.teclado.as_ref() {
            // Pasos, no porcentaje: cuatro niveles se marcan con cortes en la
            // barra y se dicen con palabras.
            let cortes: Vec<f32> = (1..t.maximo.min(6))
                .map(|i| i as f32 / t.maximo as f32)
                .collect();
            let barra_teclado = control::barra(
                ANCHO_BARRA,
                grueso(Agarre::Teclado),
                f32::from(self.porciento_teclado()) / 100.0,
                tema::acento(),
                tema::elevado(),
                &cortes,
                None,
            );
            tarjetas = tarjetas.push(self.tarjeta(
                "Teclado",
                self.valor_teclado(),
                self.ayuda_teclado().map(str::to_string),
                auto(1, 1),
                barra_teclado,
                (&self.icono_teclado, 19.0, 26.0),
                false,
            ));
        }

        let ajustes = {
            let fondo = tema::mezclar(
                tema::superficie_en_tarjeta(),
                tema::elevado(),
                self.boton.intensidad(2),
            );
            let dentro: PanelElement<'_> = match &self.icono_ajustes {
                Some(ic) => crate::icono::ver_teñido(ic, 17.0, Some(tema::texto())),
                None => Space::new().into(),
            };
            container(dentro)
                .width(Length::Fixed(AJUSTES))
                .height(Length::Fixed(AJUSTES))
                .center_x(Length::Fixed(AJUSTES))
                .center_y(Length::Fixed(AJUSTES))
                .style(move |_: &iced_widget::Theme| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: (AJUSTES / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        let cabecera = container(
            row![
                control::titulo("Brillo"),
                Space::new().width(Length::Fill),
                ajustes
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(CABECERA))
        .padding([0, SANGRIA as u16])
        .center_y(Length::Fixed(CABECERA));

        // Toda la tarjeta es el interruptor; el disco de la luna dice si está
        // encendida con el mismo lenguaje que el resto del centro de control.
        let (fondo_luna, tinta_luna) = if self.nocturna {
            control::encendido()
        } else {
            (tema::elevado(), tema::texto())
        };
        let luna = container(match &self.icono_noche {
            Some(ic) => crate::icono::ver_teñido(ic, 18.0, Some(tinta_luna)),
            None => Space::new().width(Length::Fixed(18.0)).into(),
        })
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(36.0))
        .center_x(Length::Fixed(36.0))
        .center_y(Length::Fixed(36.0))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo_luna.into()),
            border: Border {
                radius: 18.0.into(),
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
                    text(self.estado_nocturna()).size(12.0).color(tema::TEXTO2),
                ],
                Space::new().width(Length::Fill),
                lista::interruptor(self.interruptores[2].valor()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO_TARJETA))
        .height(Length::Fixed(NOCTURNA))
        .center_y(Length::Fixed(NOCTURNA))
        .padding([0, PAD_H as u16])
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(BAJO_CABECERA)),
            tarjetas,
            Space::new().height(Length::Fixed(ENTRE_TARJETAS)),
            nocturna,
        ];
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// El horario de la luz nocturna, si está en modo programado. Se lee al abrir
/// la tarjeta: cambia desde Ajustes, y entonces la tarjeta no está abierta.
// 0.1.0
fn horario_nocturna() -> Option<(tema::HoraDelDia, tema::HoraDelDia)> {
    let luz = crate::Config::cargar().luz_nocturna;
    (luz.modo == crate::ModoLuzNocturna::Programada).then_some((luz.desde, luz.hasta))
}

/// «Auto» con su mini interruptor, de 34×20. `encendido`, de 0 a 1, es el
/// recorrido de la bolita; `realce` es el puntero encima, que aclara la letra.
///
/// Reemplaza a la chapa «AUTO» y al botón redondo con una «A»: se decía tres
/// veces (chapa, botón y línea de ayuda) y la «A» se tomaba por una acción y no
/// por un modo. Un interruptor con nombre dice las dos cosas de una vez.
// añadido en la 0.2.1
fn auto_interruptor<'a>(encendido: f32, realce: f32) -> PanelElement<'a> {
    const ANCHO_PISTA: f32 = 34.0;
    const ALTO_PISTA: f32 = 20.0;
    const BOLITA: f32 = 16.0;
    let e = encendido.clamp(0.0, 1.0);
    let bolita = container(Space::new())
        .width(Length::Fixed(BOLITA))
        .height(Length::Fixed(BOLITA))
        .style(|_: &iced_widget::Theme| container::Style {
            background: Some(iced_core::Color::WHITE.into()),
            border: Border {
                radius: (BOLITA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    let pista = container(row![
        Space::new().width(Length::Fixed(
            ((ANCHO_PISTA - ALTO_PISTA) * encendido).max(0.0)
        )),
        bolita
    ])
    .width(Length::Fixed(ANCHO_PISTA))
    .height(Length::Fixed(ALTO_PISTA))
    .padding(2)
    .style(move |_: &iced_widget::Theme| container::Style {
        background: Some(tema::mezclar(tema::control_apagado(), tema::acento(), e).into()),
        border: Border {
            radius: (ALTO_PISTA / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let tinta = tema::mezclar(
        tema::mezclar(tema::TEXTO2, tema::texto(), e),
        tema::texto(),
        realce * 0.5,
    );
    container(
        row![
            text("Auto")
                .size(12.0)
                .font(control::peso(Weight::Medium))
                .color(tinta),
            Space::new().width(Length::Fixed(8.0)),
            pista,
        ]
        .align_y(Vertical::Center),
    )
    .width(Length::Fixed(AUTO))
    .height(Length::Fixed(TITULO))
    .align_right(Length::Fixed(AUTO))
    .center_y(Length::Fixed(TITULO))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El brillo no baja de un suelo: a cero no se ve ni la barra con la que
    /// volver a subirlo.
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
        b.maximo = 400;
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
    /// sola, y una segunda se saldría por encima de lo que hay debajo. La de la
    /// pantalla comparte fila con «Restablecer».
    #[test]
    fn la_ayuda_cabe_en_una_linea() {
        let interior = ANCHO_TARJETA - PAD_H * 2.0;
        for (texto, libre) in [
            ("Sigue la luz ambiente", interior - RESTABLECER),
            ("Ajustado −60 % sobre la luz", interior - RESTABLECER),
            ("Es el mínimo: más abajo no se vería.", interior),
            ("Se enciende con poca luz", interior),
        ] {
            let medido = crate::widget::ancho_de(texto, 12.0);
            assert!(medido < libre, "«{texto}» mide {medido} y caben {libre}");
        }
    }

    #[test]
    fn soltar_sin_agarrar_no_hace_nada() {
        let mut b = Brillo::new();
        assert!(!b.soltar());
    }

    /// Al mínimo la ayuda lo explica, con o sin automático: es lo que dice por
    /// qué la barra no baja más.
    #[test]
    fn el_minimo_se_explica() {
        let b = automatica(MINIMO, Some(40));
        assert_eq!(
            b.ayuda_pantalla().as_deref(),
            Some("Es el mínimo: más abajo no se vería.")
        );
    }

    /// Agarrar la barra la engorda y soltarla la devuelve; solo cuenta lo que
    /// cae en la fila, no en el hueco entre tarjetas.
    #[test]
    fn la_barra_se_agarra_en_su_fila_y_engorda() {
        let mut b = automatica(50, Some(50));
        let r = b.rect_barra(Fila::Pantalla).expect("hay barra");
        let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(b.arrastre_en(x, y));
        assert!(
            !b.arrastre_en(x, r.y + 60.0),
            "fuera de la fila no se agarra"
        );
        assert!(b.pulsar(x, y).is_none());
        assert!(b.agarrado());
        assert!(b.soltar());
        assert!(!b.agarrado());
    }

    /// «Restablecer» vuelve a lo que pide la luz, y solo existe con un ajuste
    /// que deshacer.
    #[test]
    fn restablecer_vuelve_a_lo_que_pide_la_luz() {
        let mut b = automatica(60, Some(52));
        let r = b.rect_restablecer().expect("hay ajuste de 8 puntos");
        b.pulsar(r.x + 4.0, r.y + 4.0);
        assert_eq!(b.nivel, 52);
        assert!(b.rect_restablecer().is_none(), "sin ajuste no hay enlace");
    }

    /// «Auto» solo existe con sensor y cada pulsación cambia solo su fila.
    #[test]
    fn auto_cambia_solo_su_fila() {
        let mut b = automatica(50, Some(50));
        let r = b.rect_auto(Fila::Pantalla).expect("hay sensor");
        let antes = b.automatico.teclado;
        let accion = b.pulsar(r.x + 8.0, r.y + 8.0);
        assert!(matches!(accion, Some(Accion::BrilloAutomatico(a)) if !a.pantalla));
        assert_eq!(b.automatico.teclado, antes);
        b.sensor = false;
        assert!(b.rect_auto(Fila::Pantalla).is_none());
    }

    /// El alto reservado es el que se dibuja: cada tarjeta, su hueco y la fila
    /// de la luz nocturna acaban donde acaba la emergente.
    #[test]
    fn la_geometria_cuadra_con_el_alto() {
        let mut b = automatica(50, Some(50));
        b.teclado = Some(Teclado {
            dispositivo: "kbd".into(),
            nivel: 1,
            maximo: 3,
        });
        let n = b.rect_nocturna();
        assert_eq!(n.y + n.height + MARGEN, b.size().1);
        let t = b.y_tarjeta(Fila::Teclado).expect("hay teclado");
        assert!(t > b.y_tarjeta(Fila::Pantalla).expect("siempre hay pantalla"));
        assert!(t + b.alto_teclado() + ENTRE_TARJETAS <= n.y + 0.001);
    }

    /// Los cuatro niveles del teclado se dicen con palabras.
    #[test]
    fn el_teclado_dice_palabras() {
        let mut b = Brillo::new();
        b.teclado = Some(Teclado {
            dispositivo: "kbd".into(),
            nivel: 2,
            maximo: 3,
        });
        assert_eq!(b.valor_teclado(), "Media");
        b.teclado.as_mut().expect("recién puesto").nivel = 0;
        assert_eq!(b.valor_teclado(), "Apagada");
    }
}
