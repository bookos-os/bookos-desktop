//! El aviso que sale al subir el volumen, el brillo o apagar el touchpad.
//!
//! Es el OSD de siempre: una tarjeta centrada abajo con el icono de lo que has
//! tocado y su nivel, que se va sola. Sale del mismo sitio que el resto del
//! escritorio —el shell, dentro del compositor— porque tiene que aparecer
//! **aunque no haya ninguna aplicación con foco**, y porque el compositor es
//! quien recibe las teclas de función.
//!
//! **Se desvanece por tiempo, no por un temporizador que repinte.** El
//! compositor programa un solo despertar al final y, mientras tanto, el alfa
//! sale de cuánto ha pasado: sin eso habría que repintar sesenta veces por
//! segundo durante segundo y medio para enseñar algo que no cambia. Lo único
//! que sí pide fotogramas es la entrada —180 ms— y la barra moviéndose.
//!
//! ## Uno solo, que se actualiza
//!
//! Subir el volumen cinco veces seguidas no son cinco avisos: es el mismo, con
//! la barra desplazándose y el tiempo reiniciado. Antes cada pulsación creaba
//! una cápsula nueva encima de la anterior, y la barra saltaba de un valor a
//! otro con un parpadeo en medio. Lo resuelve [`Osd::actualizar`].

use std::time::{Duration, Instant};

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, container, row, stack, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

/// Cuánto se queda a la vista antes de empezar a irse.
const QUIETO: Duration = Duration::from_millis(1200);
/// Y cuánto tarda en desvanecerse.
pub const SALIDA: Duration = Duration::from_millis(280);
/// Lo que tarda en aparecer: la duración de popover del sistema de diseño, con
/// su muelle. El aviso **es** un popover, aunque no cuelgue de nada.
const ENTRADA: Duration = tema::D_POPOVER;

/// Medidas de la cápsula, tomadas del diseño (`OSD` en Figma): 263 × 80 con el
/// radio completo, no una tarjeta con esquinas.
const ANCHO: f32 = 263.0;
pub const ALTO: f32 = 80.0;
/// Margen izquierdo hasta el icono, y del icono hasta la barra.
///
/// En el diseño el icono empieza a 32 del borde y la barra a 61; el icono mide
/// 21, así que entre los dos quedan 8.
const MARGEN_IZQ: f32 = 32.0;
const LADO_ICONO: f32 = 21.0;
const HUECO_ICONO: f32 = 8.0;
/// La barra: 171 de ancho, 9 de alto y hasta 31 del borde derecho.
const ANCHO_BARRA: f32 = 171.0;
/// Separación desde el borde de abajo hasta el **alto** de la cápsula. Va por
/// encima del dock, no encima de él: taparlo al subir el volumen es justo lo
/// que molesta del OSD de otros.
///
/// 202 y no los 170 de hace dos versiones. Con 170 la cápsula acababa a 90
/// del borde y el dock llegaba a 87 —12 de margen más sus 75 de alto—, o sea
/// tres píxeles de aire: bastaba para no solaparse cuando el aviso era un
/// rectángulo plano, pero arrastra una sombra de 24 px de desenfoque y la
/// sombra sí caía encima del dock. Se subió a 190 para dejar 23. El dock creció
/// 12 px más al repartir su padding vertical (`dock::PAD_V`) del horizontal
/// —el mismo cambio que evitó que el icono se leyera pegado a la esquina
/// redondeada—, así que esto sube con él para no perder ese aire.
/// Lo comprueba `el_aviso_no_toca_el_dock`.
pub const MARGEN_INFERIOR: f32 = 202.0;
/// Alto de la barra de nivel.
const BARRA: f32 = 9.0;

/// Margen alrededor de la cápsula, dentro del buffer, para que quepa la sombra.
///
/// La sombra se dibuja **fuera** del rectángulo del control, así que sin este
/// hueco el buffer la cortaría por los cuatro lados y se vería un borde recto
/// en vez de un degradado. Es tamaño de buffer, no de dibujo: quien coloca el
/// aviso lo **suma** a `MARGEN_INFERIOR` para que la cápsula siga donde estaba:
/// el buffer empieza más arriba justo lo que ocupa el margen de la sombra. Ver
/// [`MARGEN_BUFFER_INFERIOR`].
pub const MARGEN_SOMBRA: f32 = 18.0;

/// A qué altura del borde inferior va el **borde superior del buffer**.
///
/// Es lo que necesita quien lo coloca, y va aquí y no en el compositor porque
/// la cuenta depende de dos constantes de este módulo: sumar donde había que
/// sumar es lo que se falló al añadir la sombra, y la cápsula se metió 36 px
/// hacia abajo, encima del dock.
pub const MARGEN_BUFFER_INFERIOR: f32 = MARGEN_INFERIOR + MARGEN_SOMBRA;

/// El surco de la barra.
///
/// La parte vacía de la barra. Sólido y no el blanco al 14 % del resto del
/// sistema: la cápsula del OSD es opaca, y sobre ella un surco translúcido se
/// pierde.
///
/// En claro es el gris de surco de iOS, `#c7c7cc`. En oscuro **no** puede ser el
/// mismo: estaba así y la parte llena no se distinguía. Contraste medido de
/// los once acentos oscuros contra `#c7c7cc`: de 1,16 (turquesa) a 2,04
/// (índigo), y grafito 1,53. Contra `#3a3a3c` todos pasan del 3:1 de WCAG para
/// gráficos: el peor es el índigo, con 3,30, y el grafito sube a 4,41. El surco
/// sigue viéndose sobre la cápsula negra (1,85). Con `control_apagado`
/// (`#48484a`), el índigo se quedaba en 2,65.
// 0.1.0
fn surco() -> Color {
    if tema::es_claro() {
        tema::hex(0xc7c7cc)
    } else {
        tema::hex(0x3a3a3c)
    }
}

pub struct Osd {
    icono: Option<Icono>,
    /// Lleva encima la raya roja de «apagado»: ver [`cargar_icono`].
    tachado: bool,
    /// De 0 a 100. `None` para lo que no tiene nivel, como apagar el touchpad.
    nivel: Option<u8>,
    /// La barra, que persigue a `nivel` en vez de saltar. Guarda la fracción
    /// (0 a 1) y no el porcentaje: es lo que multiplica al ancho al dibujar.
    barra: tema::Transicion,
    /// Texto en vez de barra, para lo que no es un nivel.
    texto: Option<String>,
    /// Cuándo empezó a **entrar**. Solo se reinicia si ya se estaba yendo.
    desde: Instant,
    /// La última vez que se pulsó la tecla: de ahí cuenta el tiempo quieto.
    visto: Instant,
}

impl Osd {
    // 0.1.0
    pub fn new(icono: &str, nivel: Option<u8>, texto: Option<String>) -> Self {
        Self {
            icono: cargar_icono(icono).0,
            tachado: cargar_icono(icono).1,
            nivel,
            // Recién creado, la barra ya está en su sitio: el primer aviso
            // enseña el volumen que hay, no lo dibuja creciendo desde cero.
            barra: tema::Transicion::nueva(fraccion_nivel(nivel), tema::M_RAPIDO),
            texto,
            desde: Instant::now(),
            visto: Instant::now(),
        }
    }

    /// Reaprovecha este aviso para lo que se acaba de pulsar. `false` si no
    /// vale y hay que hacer uno nuevo.
    ///
    /// Vale mientras sea **de la misma clase**: un nivel se actualiza con otro
    /// nivel, y el aviso de «Touchpad desactivado» no se convierte en la barra
    /// del volumen a media desaparición. El icono sí cambia —el del volumen
    /// tiene tres— y el tiempo vuelve a empezar, que es lo que espera quien
    /// sigue pulsando la tecla.
    // 0.1.0
    pub fn actualizar(&mut self, icono: &str, nivel: Option<u8>, texto: Option<String>) -> bool {
        if self.nivel.is_some() != nivel.is_some() {
            return false;
        }
        // Con texto, solo se reaprovecha si dice lo mismo: dos avisos distintos
        // seguidos tienen que verse como dos.
        if nivel.is_none() && self.texto != texto {
            return false;
        }
        (self.icono, self.tachado) = cargar_icono(icono);
        self.nivel = nivel;
        self.texto = texto;
        self.barra.ir_a(fraccion_nivel(nivel));
        // Mantener la tecla llama aquí cada pocos milisegundos. Reiniciar
        // también la entrada devolvía el alfa a 0 y la escala a 0,92 en cada
        // repetición, y la cápsula parpadeaba mientras la barra subía. Solo se
        // rearma la entrada si ya había empezado a irse.
        if self.visto.elapsed() >= QUIETO {
            self.desde = Instant::now();
        }
        self.visto = Instant::now();
        true
    }

    /// Cuánto queda para que desaparezca del todo. `None` si ya se fue.
    // 0.1.0
    pub fn queda(&self) -> Option<Duration> {
        (QUIETO + SALIDA).checked_sub(self.visto.elapsed())
    }

    /// Opacidad de ahora mismo: entrando, entero mientras está quieto, y
    /// bajando al final.
    // 0.1.0
    pub fn alfa(&self) -> f32 {
        // La entrada va con la curva sin rebote aunque el tamaño sí rebote:
        // un muelle en el alfa lo llevaría por encima de 1 y habría que
        // recortarlo, gastando la parte interesante de la curva en nada.
        let entrada = tema::C_ENTRADA.eval(tema::fraccion(self.desde.elapsed(), ENTRADA));
        let quieto = self.visto.elapsed();
        if quieto < QUIETO {
            return entrada;
        }
        entrada * (1.0 - tema::C_SUAVE.eval(tema::fraccion(quieto - QUIETO, SALIDA)))
    }

    /// La escala con la que se compone: entra creciendo desde el 92 % con
    /// muelle, como los popovers del sistema, y se queda en 1.
    ///
    /// Lo aplica el compositor al componer la superficie, no el dibujo: escalar
    /// una textura ya pintada es trabajo de la GPU y no cuesta un repintado.
    // 0.1.0
    pub fn escala(&self) -> f32 {
        let t = self.desde.elapsed();
        if t >= ENTRADA {
            return 1.0;
        }
        0.92 + 0.08 * tema::C_MUELLE_POPOVER.eval(tema::fraccion(t, ENTRADA))
    }

    /// ¿Hay que repintar o recomponer? Solo mientras entra y mientras la barra
    /// se mueve; la desaparición es alfa y la lleva el compositor con un único
    /// despertar programado.
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.desde.elapsed() < ENTRADA || self.barra.animando()
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (
            self.ancho() + MARGEN_SOMBRA * 2.0,
            ALTO + MARGEN_SOMBRA * 2.0,
        )
    }

    /// El ancho de la cápsula.
    ///
    /// Con barra es fijo, el del diseño. Con texto se ajusta a lo que hay que
    /// escribir —«Touchpad desactivado» y «Fn lock» no ocupan lo mismo—, que es
    /// lo que hacen las cápsulas del diseño: 221 una y 199 otra.
    // 0.1.0
    fn ancho(&self) -> f32 {
        match (&self.texto, self.nivel) {
            (Some(t), None) => {
                // `ancho_de` y no `ancho_texto`: esa es la estimación por
                // carácter que su propio comentario dice que no vale para
                // esto —"desalineaba los clics"— y aquí desalineaba el
                // relleno. Medido: "Touchpad desactivado" estimaba 146 y mide
                // 136 de verdad; "Escritorio 2" estimaba 88 y mide 68, casi un
                // 30 % de más. La cápsula salía siempre más ancha que su
                // texto, con el icono y la frase pegados a la izquierda y un
                // hueco vacío a la derecha que no pertenecía a ningún margen.
                let texto = crate::widget::ancho_de(t, tema::T_CUERPO);
                (MARGEN_IZQ + LADO_ICONO + HUECO_ICONO + texto + MARGEN_IZQ).max(ALTO * 2.0)
            }
            _ => ANCHO,
        }
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let base = match &self.icono {
            Some(ic) => icono::ver_teñido(ic, LADO_ICONO, Some(tema::texto())),
            None => crate::widget::vacio(),
        };
        let dibujo = match icono::propio("tachado").filter(|_| self.tachado) {
            Some(raya) => stack![
                base,
                icono::ver_teñido_propio(&raya, LADO_ICONO, tema::rojo())
            ]
            .into(),
            None => base,
        };
        // El nivel manda sobre el texto: si hay barra, el número sobra — la
        // barra ya dice cuánto, y a un OSD que se va en un segundo no le sobra
        // sitio para decir lo mismo dos veces.
        let derecha: PanelElement<'_> = match (self.nivel, &self.texto) {
            (Some(_), _) => self.barra(),
            (None, Some(t)) => text(t.clone())
                .size(tema::T_CUERPO)
                .color(tema::texto())
                .into(),
            (None, None) => Space::new().into(),
        };
        let capsula = container(
            row![
                Space::new().width(Length::Fixed(MARGEN_IZQ)),
                dibujo,
                Space::new().width(Length::Fixed(HUECO_ICONO)),
                derecha,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho()))
        .height(Length::Fixed(ALTO))
        .center_y(Length::Fixed(ALTO))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo().into()),
            border: Border {
                // Cápsula, no tarjeta: es lo que dice el diseño y lo que la
                // distingue de las emergentes que cuelgan del panel.
                radius: (ALTO / 2.0).into(),
                // Un filo del color de la tinta al 10 %: sobre un fondo de
                // pantalla claro, la cápsula blanca del tema claro se quedaba
                // sin silueta y parecía un texto flotando.
                width: 1.0,
                color: tema::alfa(tema::tinta(), 0.10),
            },
            // La sombra es lo que lo despega del escritorio. La de toast es
            // difusa y clara: el aviso flota sobre cualquier cosa, y una sombra
            // dura se leería como un borde negro sobre un fondo oscuro.
            shadow: tema::sombra_toast(),
            ..Default::default()
        });
        // El margen de la sombra: el buffer es más grande que la cápsula y esto
        // la coloca en medio. Sin fondo, o sea transparente.
        container(capsula).padding(MARGEN_SOMBRA).into()
    }

    // 0.1.0
    fn barra<'a>(&self) -> PanelElement<'a> {
        // Del valor animado, no del nivel: la barra persigue lo que se acaba de
        // pulsar en vez de saltar a ello.
        let lleno = ANCHO_BARRA * self.barra.valor().clamp(0.0, 1.0);
        let relleno = container(Space::new())
            .width(Length::Fixed(lleno))
            .height(Length::Fixed(BARRA))
            .style(|_theme: &iced_widget::Theme| container::Style {
                // Acento, no blanco: en el diseño la parte llena es el azul del
                // sistema, que es además lo que distingue de un vistazo cuánto
                // hay puesto sobre un surco casi blanco.
                background: Some(tema::acento().into()),
                border: Border {
                    radius: (BARRA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(relleno)
            .width(Length::Fixed(ANCHO_BARRA))
            .height(Length::Fixed(BARRA))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(surco().into()),
                border: Border {
                    radius: (BARRA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

/// El icono de un aviso y si va tachado. Un nombre acabado en `-desactivado` es
/// el mismo icono con la raya roja encima: así «touchpad» y «cámara» tienen su
/// versión apagada sin duplicar el dibujo.
// 0.1.0
fn cargar_icono(nombre: &str) -> (Option<Icono>, bool) {
    let (base, tachado) = match nombre.strip_suffix("-desactivado") {
        Some(base) => (base, true),
        None => (nombre, false),
    };
    (icono::propio(base).or_else(|| icono::cargar(base)), tachado)
}

/// El nivel como fracción del ancho de la barra. Sin nivel, vacía.
// 0.1.0
fn fraccion_nivel(nivel: Option<u8>) -> f32 {
    nivel.unwrap_or(0).min(100) as f32 / 100.0
}

/// Fondo de la tarjeta: casi negro y bastante opaco. Aquí no hay cristal
/// detrás —el OSD se mueve y sale sobre cualquier cosa— así que el contraste
/// lo tiene que poner él.
/// La cápsula es **opaca**, como en el diseño: negra en oscuro, blanca en claro.
///
/// Estaba al 92 % de un gris muy oscuro, que es lo que se hace con las tarjetas
/// que cuelgan del panel; el OSD no cuelga de nada y sale sobre lo que haya en
/// pantalla, así que el tono pleno es lo que le da su silueta. Por eso no usa
/// `tema::card()`, que en oscuro es el gris `#1c1c1e` de las tarjetas.
// 0.1.0
fn fondo() -> Color {
    if tema::es_claro() {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mantener la tecla actualiza el aviso sin parar: el alfa y la escala no
    /// pueden volver a la entrada, que es lo que se veía como parpadeo.
    #[test]
    fn actualizar_estando_visible_no_reinicia_la_entrada() {
        let mut osd = Osd::new("brillo", Some(50), None);
        std::thread::sleep(ENTRADA);
        assert!(osd.actualizar("brillo", Some(55), None));
        assert_eq!(osd.alfa(), 1.0);
        assert_eq!(osd.escala(), 1.0);
        assert!(osd.queda().is_some_and(|q| q > QUIETO));
    }

    /// Aparece entrando, se queda entero durante su rato y se va al final.
    #[test]
    fn entra_se_queda_y_se_va() {
        let osd = Osd::new("volumen-alto", Some(50), None);
        assert!(osd.alfa() < 1.0, "recién creado está entrando");
        std::thread::sleep(ENTRADA);
        assert_eq!(osd.alfa(), 1.0, "pasada la entrada se ve entero");
        assert!(osd.queda().is_some());
    }

    /// El nivel se recorta: un volumen del 150 % no puede pintar una barra que
    /// se salga de la cápsula.
    /// El aviso no puede tocar el dock: es lo que más molesta del OSD de otros
    /// escritorios, y es lo que pasó al darle sombra —el buffer creció por los
    /// cuatro lados y la cápsula bajó con él.
    ///
    /// Se mide desde el borde inferior de la pantalla hacia arriba, que es como
    /// se colocan los dos.
    #[test]
    fn el_aviso_no_toca_el_dock() {
        let alto_dock = crate::dock::Dock::from_config(&[]).size().1;
        let techo_del_dock = crate::dock::MARGIN + alto_dock;
        let base_de_la_capsula = MARGEN_INFERIOR - ALTO;
        let aire = base_de_la_capsula - techo_del_dock;
        // No basta con que no se solapen: la sombra tiene 24 px de desenfoque y
        // se ve mucho antes de que los rectángulos se toquen.
        assert!(
            aire >= 16.0,
            "el aviso se le echa encima al dock: {aire} px de aire"
        );
    }

    #[test]
    fn la_barra_no_se_desborda() {
        let osd = Osd::new("volumen-alto", Some(200), None);
        assert_eq!(fraccion_nivel(Some(200)), 1.0);
        assert_eq!(osd.barra.valor(), 1.0);
        assert!(osd.queda().is_some());
    }

    /// Subir el volumen otra vez no crea un aviso nuevo: mueve el que hay y le
    /// reinicia el tiempo. Lo contrario era una cápsula parpadeando por cada
    /// pulsación de la tecla.
    #[test]
    fn el_mismo_aviso_se_reaprovecha() {
        let mut osd = Osd::new("volumen-bajo", Some(20), None);
        std::thread::sleep(Duration::from_millis(20));
        assert!(osd.actualizar("volumen-alto", Some(80), None));
        assert_eq!(osd.nivel, Some(80));
        // La barra **no** está ya en el destino: va hacia él.
        assert!(
            osd.barra.valor() < 0.8,
            "la barra ha saltado: {}",
            osd.barra.valor()
        );
        assert!(osd.animando());

        // Y lo que no es de la misma clase no se reaprovecha.
        assert!(!osd.actualizar("touchpad", None, Some("Touchpad desactivado".into())));
        let mut texto = Osd::new("touchpad", None, Some("Touchpad desactivado".into()));
        assert!(!texto.actualizar("teclado", None, Some("Fn lock".into())));
        assert!(texto.actualizar("touchpad", None, Some("Touchpad desactivado".into())));
    }

    /// Entra creciendo y acaba a tamaño natural. Que termine en 1,0 exacto
    /// importa: el compositor compara con 1,0 para saltarse la capa de escalado.
    #[test]
    fn entra_creciendo_y_se_queda_quieto() {
        let osd = Osd::new("volumen-alto", Some(50), None);
        assert!(osd.escala() < 1.0, "tiene que empezar más pequeño");
        assert!(osd.alfa() < 1.0, "y entrando");
        std::thread::sleep(ENTRADA);
        assert_eq!(osd.escala(), 1.0);
        assert_eq!(osd.alfa(), 1.0);
        assert!(!osd.animando());
    }
}

#[cfg(test)]
mod pruebas_ancho_texto {
    use super::*;

    /// La cápsula de un OSD de texto tiene que ceñirse a lo que dice, no a una
    /// estimación por carácter.
    ///
    /// Antes usaba `ancho_texto` —7,3 px por carácter—, que ese mismo módulo
    /// documenta como no válida para nada que tenga que cuadrar con el dibujo.
    /// Con "Escritorio 2" se notaba: la cápsula salía casi un 30 % más ancha
    /// que el texto, con el icono y la frase pegados a la izquierda y un
    /// hueco vacío a la derecha.
    #[test]
    fn la_capsula_de_texto_se_ciñe_al_texto_de_verdad() {
        for t in [
            "Touchpad desactivado",
            "Touchpad activado",
            "Escritorio 2",
            "Escritorio 10",
            "Fn lock",
        ] {
            let o = Osd::new("touchpad", None, Some(t.to_string()));
            let ancho_capsula = o.ancho();
            let real = crate::widget::ancho_de(t, tema::T_CUERPO);
            let esperado =
                (MARGEN_IZQ + LADO_ICONO + HUECO_ICONO + real + MARGEN_IZQ).max(ALTO * 2.0);
            assert!(
                (ancho_capsula - esperado).abs() < 0.5,
                "{t:?}: cápsula={ancho_capsula:.1} esperado={esperado:.1}"
            );
        }
    }
}
