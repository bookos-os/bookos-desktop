//! La isla de actividades vivas de las aplicaciones de sistema.
//!
//! No es una notificación: permanece mientras la tarea siga viva, tiene
//! controles y solo recibe estado del contrato privado de BookOS. La forma se
//! pinta en CPU una vez; la entrada y el cambio de estado los compone la GPU.
//!
//! ## Cómo está construida la cara
//!
//! - **Superficie de tarjeta, lisa.** Blanca en claro y carbón en oscuro
//!   ([`tema::card`]), como cualquier tarjeta del sistema de diseño. Ninguna
//!   actividad va rellena de color: el temporizador y el calendario lo iban, y
//!   una tarjeta entera de acento decía «esto se toca» por toda su superficie.
//!   El color se queda en lo que se pulsa (acento) y en lo que informa de un
//!   estado (rojo de grabar, ámbar del temporizador, verde de la transferencia).
//! - **La geometría vive en constantes, no en el árbol de widgets.** Las zonas
//!   sensibles al clic se calculan a mano en [`Actividad::pulsar`] —aquí no hay
//!   eventos de iced, el compositor entrega un punto— así que un `padding` que
//!   se cambia en la vista y no en la hitbox deja el botón donde no se ve. Con
//!   las medidas en un solo sitio, mover un control mueve las dos cosas.
//! - **Los pictogramas son SVG, no glifos.** Estaban escritos como texto
//!   («‹‹», «Ⅱ», «♥»), y eso depende de que la fuente del sistema los tenga:
//!   medido, el corazón de «favorita» no se dibujaba **nada** en esta máquina y
//!   la fila de la cola salía vacía en ese hueco.
//! - **El acento manda en los controles, no la portada.** Teñirlos con el
//!   color del disco hacía que el botón de reproducir cambiase de color con
//!   cada canción y dejase de leerse como el control del sistema.
//!
//! ## Pendiente: inglés
//!
//! Los textos propios de la isla están **solo en español**: este fichero no
//! pasa ninguno por [`crate::t`], al contrario que el resto de emergentes, así
//! que con `LANG` en inglés sale igual. Son unos 25: grabadora, temporizador,
//! calendario (incluidos los `format!` de `cuenta_atras` y `formato_corto`,
//! que necesitan su rama por idioma), Zenith, «A continuación» de la música y
//! los botones. El título y el subtítulo no entran: los manda la aplicación
//! que publica la actividad, en su idioma.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::{Font, Weight};
use iced_core::{Border, Color, Length, Shadow, Vector};
use iced_widget::{Row, Space, column, container, image, row, stack, text};

use crate::icono;
use crate::tema;
use crate::view::PanelElement;

/// Aire alrededor de la tarjeta para que quepa la sombra. El compositor lo
/// resta al colocar la superficie, así que subirlo no mueve la isla.
pub const MARGEN_SOMBRA: f32 = 26.0;
/// Distancia del canto superior de la tarjeta al borde de la pantalla. La isla
/// no cuelga por debajo del panel sino que nace **dentro** de él, en el centro:
/// la píldora compacta del reproductor queda centrada en la barra de 32 px
/// (3 + 26 + 3) y, al abrirse, la tarjeta crece hacia abajo desde ahí.
pub const ARRIBA: f32 = 3.0;

// ── Geometría ────────────────────────────────────────────────────────────
// Todas las medidas del interior de la tarjeta. Ver la nota del módulo: estas
// constantes las comparten la vista y las zonas de clic.

/// Las tres píldoras cerradas caben en la barra superior y tienen su alto;
/// cambia el ancho según lo que lleva cada una. Tiene que estar en `size` y no
/// solo en la vista, porque el compositor centra **el buffer** en la pantalla
/// y una píldora más estrecha dentro de un buffer ancho se vería descentrada.
const TIMER_COMPACTO_W: f32 = 132.0;
const RECORDER_COMPACTO_W: f32 = 172.0;
/// Transferencia, calendario y monitor: mismas píldoras bajas que la música,
/// con el ancho justo para «nombre + dato».
const TRANSFER_COMPACTO_W: f32 = 236.0;
const CALENDARIO_COMPACTO_W: f32 = 290.0;
const MONITOR_COMPACTO_W: f32 = 276.0;
/// Auriculares: icono, nombre y la batería más baja, como la transferencia.
const AURICULARES_COMPACTO_W: f32 = TRANSFER_COMPACTO_W;
/// El asistente cuelga de la barra como un notch. Eran 440×52 con dos líneas:
/// la segunda repetía «Zenith · Te escucho…» junto a un orbe y una onda que ya
/// lo dicen, y tapaba un tercio de la barra. Con una línea, 300 de ancho dejan
/// unos 165 de texto, y al transcribir se ve el final de la frase.
// añadido en la 0.2.0
// modificado en la 0.2.0 -- más estrecha y de una línea.
const ASIS_COMPACTO_W: f32 = 300.0;
const ASIS_COMPACTO_H: f32 = 40.0;
const ASIS_ORBE: f32 = 24.0;
const ASIS_CERRAR_PILDORA: f32 = 26.0;
/// El canto de color que gira alrededor de la píldora.
const ASIS_CANTO: f32 = 1.5;
/// Radianes por segundo del color. Escuchando es casi quieto para que mande la
/// voz; pensando no hay voz, así que el movimiento es lo que dice «trabajando».
const ASIS_GIRO_ESCUCHA: f32 = 0.7;
const ASIS_GIRO_PIENSA: f32 = 2.6;
const ASIS_CERRAR: f32 = 32.0;
const ASIS_LADO: f32 = 10.0;
const ASIS_SEP: f32 = 12.0;
const ASIS_ONDA_W: f32 = 27.0;
/// La sacudida del error: dos vaivenes y medio que se apagan. 360 ms y 10 px
/// son la horquilla de una sacudida de error (300–400 ms, 10–15 px) por lo
/// bajo: la tarjeta es grande y más amplitud la despegaba de la barra.
const D_SACUDIDA: std::time::Duration = std::time::Duration::from_millis(360);
const SACUDIDA_PX: f32 = 10.0;
thread_local! {
    /// Se está pintando el contenido de un relevo: la caja común pone el fondo
    /// y la sombra, y las tarjetas de dentro no deben traer los suyos o su
    /// sombra se vería dentro del recorte. Como `tema::pintando_isla_clara`,
    /// es estado del pintado y no de la actividad.
    // añadido en la 0.2.3
    static SIN_FONDO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// añadido en la 0.2.3
fn sin_fondo() -> bool {
    SIN_FONDO.with(std::cell::Cell::get)
}

/// El paso de una actividad a otra con la rueda o las flechas.
///
/// Antes la tarjeta cambiaba de tamaño y de contenido de un fotograma a otro,
/// con un empujón de escala encima: un salto, y el HIG (§8) pide continuidad,
/// cambios de forma con `M_ISLA` y nunca saltar. Ahora la caja va del tamaño
/// de la anterior al de la nueva con el muelle de la isla, y el contenido se
/// relee como en la maqueta de la isla (artifact de diseño, `.cap`): el viejo
/// se apaga en [`RELEVO_SALE`] y el nuevo entra cuando la forma ya casi ha
/// llegado, tras [`RELEVO_ESPERA`].
// añadido en la 0.2.3
// modified 0.2.3 -- fundido escalonado en vez de desplazar en vertical: con
// el contenido moviéndose a la vez que la caja se leían dos movimientos.
struct Relevo {
    /// `None` cuando es la misma actividad cambiando de vista (abrir, cola,
    /// menú ⋯): solo cambia la caja, el contenido nuevo no se desplaza.
    // modificado en la 0.2.3
    previa: Option<Box<Actividad>>,
    desde: Instant,
    /// Tamaño de la tarjeta (sin el margen de sombra) y radio de donde parte.
    tam0: (f32, f32),
    radio0: f32,
    /// `true` cuando la misma tarjeta solo se extiende o recoge (p. ej. abrir o
    /// cerrar la cola «A continuación»): el contenido superior permanece
    /// visible sin apagarse ni fundirse, recortado por el alto actual.
    // añadido en la 0.2.4
    extender: bool,
}

/// Lo que tarda en apagarse el contenido que se va (maqueta: `.cap`, 100 ms).
// added 0.2.3
const RELEVO_SALE: Duration = Duration::from_millis(100);
/// Cuándo empieza a entrar el nuevo: a los 190 ms la caja de `D_ISLA` lleva
/// ~85 % del recorrido y el contenido ya no aparece recortado por ella.
// added 0.2.3
const RELEVO_ESPERA: Duration = Duration::from_millis(190);
/// Lo que tarda en encenderse el contenido nuevo (maqueta: 200 ms).
// added 0.2.3
const RELEVO_ENTRA: Duration = Duration::from_millis(200);
/// El empujón al pasar de actividad (maqueta: `paso`, 260 ms): baja 2 px y
/// crece un 3 % hasta el 40 % del tiempo y vuelve. Dice «otra» sin mover el
/// contenido.
// added 0.2.3
const PASO: Duration = Duration::from_millis(260);

/// Constante de tiempo con que la onda y el brillo siguen la voz. Zenith manda
/// el nivel unas siete veces por segundo; sin suavizar, cada aviso era un
/// escalón. 90 ms lo alcanza antes del siguiente sin parecer que llega tarde.
const NIVEL_TAU: f32 = 0.09;
/// Cuánto crece la píldora con la voz a tope. Más de un 4 % despegaba el
/// canto de la barra y la píldora parecía saltar, no respirar.
const HINCHADO_VOZ: f32 = 0.04;
/// Minimizado: una burbuja flotante con la onda dentro, como la de Gemini. El
/// compositor la coloca donde se suelte, no en la barra.
pub const ASIS_MINI: f32 = 52.0;
/// La cabecera reserva la misma altura que la zona de cierre.
const ASIS_MARCA_H: f32 = ASIS_CERRAR;
const ASIS_TITULO_H: f32 = 24.0;
const ASIS_LINEA_H: f32 = 19.5;
const ASIS_CUERPO_MAX: usize = 6;
/// La música cerrada es una píldora baja: una canción de fondo no merece la
/// tarjeta de 380×72 del temporizador, que tapaba las pestañas de la ventana
/// de debajo. Una línea de título y el ecualizador bastan para reconocerla; el
/// resto está a un clic.
/// La música compacta no lleva título: carátula a la izquierda y ondas a la
/// derecha, como la isla de la referencia (Figma «Spotify Dynamic Island»).
/// El título está a un clic, en la abierta.
// modified 0.2.0 -- 280 → 150, sin título
const PLAYER_COMPACTO_W: f32 = 150.0;
/// Lo que cabe en la barra superior dejando `ARRIBA` por encima y por debajo.
const PLAYER_COMPACTO_H: f32 = crate::PANEL_HEIGHT as f32 - ARRIBA * 2.0;
/// La isla cerrada es un squircle, no una píldora: con la mitad del alto de
/// radio se leía como un botón de cápsula. 10 es el radio de «botón pequeño» de
/// la tabla del HIG, y a estos 26 px de alto deja esquinas claramente cuadradas.
const RADIO_COMPACTO: f32 = 10.0;
/// La superficie de la isla, en el tono de control del tema (maqueta «Panel e
/// isla»): en oscuro es `card()` (`#1C1C1E`: sobre el panel negro ya se ve
/// dónde pulsar, y el negro puro desaparecía); en claro, `#D8D8DD`, un gris
/// que la separa del panel blanco sin filo. Antes era negra en los dos temas y
/// su texto el del tema oscuro, forzado al pintar (`Shell::draw_actividad`).
// modificado en la 0.2.3
fn isla() -> Color {
    tema::card()
}
/// Radio de la tarjeta abierta con su columna: el de diálogo, el mayor de la
/// tabla del HIG (§4).
const RADIO_ABIERTO_ISLA: f32 = tema::R_DIALOGO;
/// La columna de actividades a la derecha de la tarjeta abierta: la actual a 30
/// y las demás a 18, con 6 entre ellas. Va en su propio hueco, que ensancha la
/// tarjeta, para que nunca pise el contenido.
// añadido en la 0.2.0
const COLUMNA: f32 = 48.0;
const COLUMNA_ACTUAL: f32 = 30.0;
const COLUMNA_OTRA: f32 = 18.0;
const COLUMNA_SEP: f32 = 6.0;
/// Hueco del «+N» de la compacta: cabe «+9» a 11 px en negrita con su
/// separación; más de nueve vivas no se dan.
// añadido en la 0.2.0
const CONTADOR_W: f32 = 24.0;
const ABIERTO_W: f32 = 440.0;
/// Cabecera de las tarjetas abiertas: «App ›» a la izquierda, que es el único
/// sitio que abre la aplicación, y ⋯ a la derecha. Va en una franja propia
/// encima del contenido; el fondo de la tarjeta no abre nada, para que un toque
/// suelto del touchpad al acabar de deslizar no saque la app sin querer.
// añadido en la 0.2.2
const CAB_APP_Y: f32 = 10.0;
const CAB_APP_H: f32 = 26.0;
const CAB_APP_X: f32 = 14.0;
/// Lo que crece la tarjeta por la cabecera: el contenido empieza a 46 del
/// canto, como en la maqueta.
const CAB_EXTRA: f32 = 24.0;
const CAB_ICONO: f32 = 18.0;
const MAS_W: f32 = 30.0;
const MAS_H: f32 = 24.0;
const MAS_DER: f32 = 12.0;
/// Filas del menú ⋯: el popover del HIG, 14,5/500 con 11 de relleno vertical.
const MENU_FILA_H: f32 = 40.0;
const MENU_FILAS: usize = 4;
/// Margen interior de las vistas abiertas.
const PAD: f32 = 22.0;

/// Rótulo de sección («Temporizador», «Rendimiento»): el section header del
/// sistema de diseño, 12/Medium en texto secundario.
const ROTULO_H: f32 = 16.0;
/// Botón ancho de la tarjeta: el botón de diálogo del sistema de diseño.
const BOTON_H: f32 = 48.0;
const BOTON_SEP: f32 = 12.0;
/// Barras de progreso de todas las tarjetas.
const BARRA_GRUESO: f32 = 6.0;

/// Reproductor abierto, de arriba abajo.
const ARTE: f32 = 66.0;
const CAB_H: f32 = ARTE;
const CTRL_Y: f32 = PAD + CAB_H + 18.0;
const CTRL_H: f32 = 56.0;
/// Los cinco controles: aleatorio, anterior, reproducir, siguiente, repetir.
const CTRL_PLAY: f32 = 56.0;
const CTRL_LADO: f32 = 44.0;
const CTRL_PLANO: f32 = 26.0;
const CTRL_SEP: f32 = 12.0;
/// Fila del progreso: tiempo, barra, tiempo.
const BARRA_Y: f32 = CTRL_Y + CTRL_H + 16.0;
const BARRA_H: f32 = 20.0;
/// Ancho reservado a cada rótulo de tiempo dentro de esa fila. Fijo para que
/// la barra no cambie de largo al pasar de «9:59» a «10:00».
const TIEMPO_W: f32 = 42.0;
/// Fila secundaria, bajo un divisor: la cola en la mitad izquierda y el
/// volumen en la derecha.
const SEC_Y: f32 = BARRA_Y + BARRA_H + 12.0 + 1.0 + 10.0;
const SEC_H: f32 = 28.0;
const ICONO_FILA: f32 = 20.0;
/// La pista del volumen: empieza tras su icono y acaba en el margen. El
/// pulgar entra entero en los dos extremos, así que el recorrido útil es el
/// ancho menos el pulgar.
const VOL_X0: f32 = ABIERTO_W / 2.0 + 16.0 + ICONO_FILA + 10.0;
const VOL_X1: f32 = ABIERTO_W - PAD;
const VOL_PULGAR: f32 = 20.0;
const PLAYER_H: f32 = SEC_Y + SEC_H + PAD;

/// La cola cuelga por debajo del reproductor, dentro de la misma tarjeta.
const COLA_SEP: f32 = 10.0;
const COLA_CAB_H: f32 = 30.0;
const COLA_FILA_H: f32 = 60.0;
const COLA_PAD: f32 = 14.0;
const COLA_MAX: usize = 4;

/// Temporizador abierto: rótulo, cifra, estado, barra y botones.
const CIFRA_H: f32 = 54.0;
const ESTADO_H: f32 = 18.0;
const TIMER_BOTON_Y: f32 =
    PAD + ROTULO_H + 6.0 + CIFRA_H + 2.0 + ESTADO_H + 14.0 + BARRA_GRUESO + 18.0;
const TIMER_H: f32 = TIMER_BOTON_Y + BOTON_H + PAD;

/// Grabadora abierta: estado, tiempo, ondas y los dos botones redondos.
const REC_CAB_H: f32 = 20.0;
const REC_TIEMPO_H: f32 = 40.0;
const REC_ONDAS_H: f32 = 46.0;
const REC_BOTON_Y: f32 = PAD + REC_CAB_H + 6.0 + REC_TIEMPO_H + 12.0 + REC_ONDAS_H + 18.0;
const REC_BOTON: f32 = 56.0;
const RECORDER_H: f32 = REC_BOTON_Y + REC_BOTON + PAD;

/// Transferencia abierta: archivo, barra, «42 % … Quedan 1:12» y cancelar.
const TRANSFER_CAB_H: f32 = 44.0;
const TRANSFER_INFO_H: f32 = 16.0;
const TRANSFER_BOTON_Y: f32 =
    PAD + TRANSFER_CAB_H + 18.0 + BARRA_GRUESO + 10.0 + TRANSFER_INFO_H + 18.0;
const TRANSFER_H: f32 = TRANSFER_BOTON_Y + BOTON_H + PAD;

/// Calendario abierto: rótulo, evento, horario, cuenta atrás y un botón.
const CAL_TITULO_H: f32 = 28.0;
const CAL_SUB_H: f32 = 18.0;
const CAL_CUANDO_H: f32 = 20.0;
const CAL_BOTON_Y: f32 =
    PAD + ROTULO_H + 6.0 + CAL_TITULO_H + 2.0 + CAL_SUB_H + 12.0 + CAL_CUANDO_H + 18.0;
const CAL_H: f32 = CAL_BOTON_Y + BOTON_H + PAD;

/// Monitor abierto: una fila «nombre … cifra» por métrica, sin barras ni
/// botones; pulsar cierra.
const MON_FILA_H: f32 = 36.0;

/// Auriculares abiertos: el aparato, las filas del monitor (izquierdo,
/// derecho, estuche) y debajo un botón para Ajustes.
const AUR_CAB_H: f32 = 32.0;
const AUR_BOTON_Y: f32 = PAD + AUR_CAB_H + 8.0 + MON_FILA_H * 3.0 + 16.0;
const AUR_H: f32 = AUR_BOTON_Y + BOTON_H + PAD;

/// Lado en píxeles de la carátula ya preparada: tres por punto del mayor
/// tamaño al que se pinta (66), nítida hasta escala 3.
const ARTE_PX: u32 = 198;
/// Lo mismo para las miniaturas de 42 puntos de la cola.
const MINIATURA_PX: u32 = 126;

/// Radio de las tarjetas abiertas. Más generoso que el de una tarjeta normal:
/// la referencia del sistema de diseño para la isla es casi una píldora.
/// El radio de diálogo del sistema de diseño (tabla cerrada de la sección
/// 2.3): la tarjeta abierta es una superficie temporal por encima de todo.
const RADIO: f32 = 26.0;

/// Las ondas son un indicador ambiental, no una visualización de audio de
/// precisión. Redibujar y subir el bitmap completo de la tarjeta a 120 Hz
/// hacía competir a iced con el cursor. A 12,5 Hz conservan movimiento en la
/// píldora compacta sin monopolizar el hilo de entrada en builds de desarrollo.
const FRAME_ONDAS: std::time::Duration = std::time::Duration::from_millis(80);
/// A 80 ms el color que gira se ve a saltos. La píldora de Zenith es corta y
/// solo está mientras se le habla: puede permitirse 30 repintados por segundo.
const FRAME_ASISTENTE: std::time::Duration = std::time::Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    Player,
    Timer,
    Recorder,
    /// Un archivo que se está enviando o recibiendo. `nivel` es el progreso
    /// (0..1) y `restante_ms` lo que falta, −1 si no se sabe.
    Transfer,
    /// Un aviso de clase o evento. `restante_ms` es lo que falta para que
    /// empiece; negativo, ya ha empezado.
    Calendar,
    /// Rendimiento rápido: CPU, GPU, RAM, temperatura y batería en `metricas`.
    Monitor,
    /// La batería de unos auriculares al sacarlos: una métrica por auricular
    /// y otra para el estuche si informa. `titulo` es el nombre del aparato.
    Auriculares,
    /// Zenith. `titulo` es lo que se oye o el titular de la respuesta,
    /// `subtitulo` el estado o el cuerpo y `nivel` la voz (0..1).
    // añadido en la 0.2.0
    Asistente(Fase),
}

/// En qué está el asistente. Lo decide Zenith; la isla solo lo dibuja y el
/// compositor pone el brillo de borde mientras escucha o piensa.
// añadido en la 0.2.0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase {
    Escuchando,
    Pensando,
    /// Respuesta que hay que leer: la isla se abre en tarjeta.
    Respuesta,
    /// Orden directa ya hecha («brillo al 50 %»): una línea y se va sola.
    Hecho,
    /// Algo impide seguir; el cuerpo dice por qué y el botón reintenta.
    Error,
    /// Sigue escuchando sin ocupar la barra.
    Minimizado,
}

impl Fase {
    // añadido en la 0.2.0
    pub fn desde(nombre: &str) -> Option<Self> {
        Some(match nombre {
            "escuchando" | "" => Self::Escuchando,
            "pensando" => Self::Pensando,
            "respuesta" => Self::Respuesta,
            "hecho" => Self::Hecho,
            "error" => Self::Error,
            "minimizado" => Self::Minimizado,
            _ => return None,
        })
    }

    /// Si el escritorio lleva el brillo de borde: solo mientras se le habla o
    /// trabaja, que es cuando hay que saber que está ahí. Leyendo la respuesta
    /// ya no aporta nada y minimizado se sigue trabajando encima.
    // añadido en la 0.2.0
    pub fn con_brillo(self) -> bool {
        matches!(self, Self::Escuchando | Self::Pensando)
    }

    // añadido en la 0.2.0
    fn abierta(self) -> bool {
        matches!(self, Self::Respuesta | Self::Error)
    }
}

/// Una fila del monitor. `critica` la decide quien publica: una batería al 10 %
/// es grave y una CPU al 10 % no, y de eso la isla no sabe nada.
#[derive(Debug, Clone)]
pub struct Metrica {
    pub etiqueta: String,
    pub valor: f32,
    pub texto: String,
    pub critica: bool,
}

#[derive(Debug, Clone)]
pub struct Portada {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct ItemCola {
    pub id: String,
    pub titulo: String,
    pub artista: String,
    /// Duración de la pista. `0` si la aplicación no la manda: entonces no se
    /// enseña, en vez de escribir un «00:00» que no significa nada.
    pub duracion_ms: i64,
    pub favorita: bool,
    pub actual: bool,
    /// Carátula de la pista. La aplicación solo la manda cuando cambia la
    /// cola; entre medias llega `None` y se reutiliza la ya preparada.
    pub portada: Option<Portada>,
}

#[derive(Debug, Clone)]
pub struct Estado {
    pub app_id: String,
    pub clase: Clase,
    pub activo: bool,
    pub pausado: bool,
    pub titulo: String,
    pub subtitulo: String,
    pub posicion_ms: i64,
    pub duracion_ms: i64,
    pub restante_ms: i64,
    pub volumen: u8,
    pub nivel: f32,
    /// Reproducción aleatoria y repetición, para poder pintar encendidos sus
    /// dos botones. Son estado de la aplicación: el compositor no los decide.
    pub aleatorio: bool,
    pub repetir: bool,
    /// Control de ruido de unos auriculares: 0 apagado, 1 cancelación,
    /// 2 ambiente (los modos de `bookos-budsd`). `None` si no lo publican.
    // añadido en la 0.2.2
    pub anc: Option<u8>,
    pub portada: Option<Portada>,
    pub cola: Vec<ItemCola>,
    pub metricas: Vec<Metrica>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accion {
    pub app_id: String,
    pub nombre: String,
    pub valor: String,
}

/// Otra actividad viva que no es la principal. Se pinta como un círculo con
/// su color y se pulsa para pasarla a la píldora.
#[derive(Debug, Clone)]
pub struct Burbuja {
    pub app_id: String,
    pub clase: Clase,
    pub color: Color,
    pub activa: bool,
    /// Es la que se ve: en la columna va grande y a todo color.
    // añadido en la 0.2.0
    pub actual: bool,
}

/// Cómo empieza la animación que arranca en `desde`.
// añadido en la 0.2.0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entrada {
    /// Nace de la barra: del 60 % y transparente.
    Nueva,
    /// La misma isla cambia de vista en su sitio (la píldora de Zenith se
    /// despliega en tarjeta): parte casi entera y medio opaca. Nacer de cero
    /// la hacía parpadear, desaparecer y volver a crecer, justo al llegar la
    /// respuesta.
    OtraVista,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vista {
    Compacta,
    Abierta,
    Cola,
    /// El menú ⋯ de la actividad, en lugar del contenido.
    // añadido en la 0.2.2
    Menu,
}

pub struct Actividad {
    estado: Estado,
    vista: Vista,
    animaciones: bool,
    desde: Instant,
    ultimo_frame_ondas: Instant,
    /// La portada con las esquinas ya recortadas.
    ///
    /// Se prepara al recibir el estado y no al dibujar: recortar son 90 000
    /// píxeles por portada, y hacerlo en cada fotograma de una animación de
    /// 220 ms es pagarlo trece veces para ver lo mismo.
    arte: Option<image::Handle>,
    /// Miniaturas de la cola por id de pista, ya recortadas y reducidas.
    miniaturas: HashMap<String, image::Handle>,
    burbujas: Vec<Burbuja>,
    /// `--blue`, `--accent-2` y `--accent-3` de la paleta del fondo, para el
    /// asistente. Se leen al recibir estado, no al pintar cada paso de la onda.
    // añadido en la 0.2.0
    tonos: [Color; 3],
    /// `--on-accent`: la tinta sobre el primer tono, para el botón.
    sobre_tono: Color,
    /// Se parte al publicar: la altura y la zona del botón deben usar las
    /// mismas líneas que se pintan, también con saltos de línea explícitos.
    cuerpo_asistente: String,
    /// Desde cuándo se va. La aplicación ya la retiró, pero se queda a la
    /// vista lo que dura el fundido: desaparecer de golpe parecía un fallo.
    // añadido en la 0.2.0
    saliendo: Option<Instant>,
    /// Último cambio de clase sin cambiar de vista (Zenith pasa de escuchar a
    /// pensar): un pulso corto para que se note que ha entendido.
    // añadido en la 0.2.0
    cambio: Option<Instant>,
    /// Desde cuándo está en error. La tarjeta se sacude al aterrizar: un
    /// error con el mismo pulso que un «Listo» no se distinguía de reojo.
    // añadido en la 0.2.0
    sacudida: Option<Instant>,
    /// Desde qué nivel de voz y cuándo se va hacia el último publicado. Ver
    /// [`Actividad::nivel_suave`].
    // añadido en la 0.2.0
    nivel_desde: (f32, Instant),
    // añadido en la 0.2.0
    entrada: Entrada,
    /// La actividad de la que se viene, mientras se desplaza. Ver [`Relevo`].
    // añadido en la 0.2.3
    relevo: Option<Relevo>,
    /// El nombre de la aplicación para la cabecera, sacado de su `.desktop`
    /// una vez al nacer: leerlo en cada pintado son `stat` por fotograma.
    // añadido en la 0.2.2
    app_nombre: String,
}

impl Actividad {
    // 0.1.0
    // modified 0.1.1 -- se queda la portada sin copiarla
    pub fn nueva(mut estado: Estado, animaciones: bool) -> Self {
        let arte = preparar_portada(estado.portada.take().as_ref(), ARTE_PX);
        let mut a = Self {
            estado,
            vista: Vista::Compacta,
            animaciones,
            desde: Instant::now(),
            ultimo_frame_ondas: Instant::now() - FRAME_ONDAS,
            arte,
            miniaturas: HashMap::new(),
            burbujas: Vec::new(),
            tonos: [Color::BLACK; 3],
            sobre_tono: Color::WHITE,
            cuerpo_asistente: String::new(),
            saliendo: None,
            cambio: None,
            sacudida: None,
            nivel_desde: (0.0, Instant::now()),
            entrada: Entrada::Nueva,
            relevo: None,
            app_nombre: String::new(),
        };
        a.app_nombre = nombre_app(&a.estado);
        a.guardar_miniaturas();
        a.seguir_fase(true);
        a
    }

    /// Prepara las carátulas nuevas de la cola y olvida las de pistas que ya no
    /// están en ella. Las que llegan vacías conservan la que había: el
    /// reproductor no las reenvía en cada latido.
    // 0.1.0
    fn guardar_miniaturas(&mut self) {
        let cola = &mut self.estado.cola;
        self.miniaturas
            .retain(|id, _| cola.iter().any(|i| &i.id == id));
        for (i, item) in cola.iter_mut().enumerate() {
            // Se sueltan los píxeles originales: ya están en la miniatura y
            // guardarlos multiplicaría la memoria por pista. Solo se preparan
            // las filas que se pintan; reducir veinte portadas en el hilo del
            // compositor al cambiar de canción es trabajo que nadie ve.
            //
            // Ni las que ya tienen miniatura: el estado guardado las hereda de
            // una publicación a otra y llegan aquí en cada latido.
            if let Some(p) = item.portada.take()
                && i < COLA_MAX
                && !self.miniaturas.contains_key(&item.id)
                && let Some(h) = preparar_portada(Some(&p), MINIATURA_PX)
            {
                self.miniaturas.insert(item.id.clone(), h);
            }
        }
    }

    // 0.1.1
    pub fn tiene_arte(&self) -> bool {
        self.arte.is_some()
    }

    // 0.1.1
    pub fn invalidar_arte(&mut self) {
        self.arte = None;
    }

    // 0.1.1
    pub fn tiene_miniatura(&self, id: &str) -> bool {
        self.miniaturas.contains_key(id)
    }

    // 0.1.0
    // modified 0.1.1 -- solo prepara la portada si llega una nueva, sin comparar píxeles
    pub fn actualizar(&mut self, mut estado: Estado) {
        // La portada solo se vuelve a preparar si de verdad cambió: un
        // reproductor manda estado cada segundo para mover la barra de
        // progreso, y con la misma carátula.
        if let Some(portada) = estado.portada.take() {
            self.arte = preparar_portada(Some(&portada), ARTE_PX);
        } else if self.estado.titulo != estado.titulo && self.arte.is_some() {
            // Si el reproductor limpió explícitamente la carátula al cambiar de pista.
            self.arte = None;
        }
        let vista = self.vista;
        let cambia = self.estado.clase != estado.clase;
        // La burbuja minimizada vive en otro sitio de la pantalla: ir o volver
        // de ella con el pulso de un cambio de fase era teletransportarse.
        let minimizada = |c: Clase| c == Clase::Asistente(Fase::Minimizado);
        let muda = minimizada(self.estado.clase) != minimizada(estado.clase);
        self.nivel_desde = (self.nivel_suave(), Instant::now());
        self.estado = estado;
        self.saliendo = None;
        self.guardar_miniaturas();
        self.seguir_fase(cambia);
        if muda {
            self.desde = Instant::now();
            self.entrada = Entrada::Nueva;
            self.cambio = None;
        } else if self.vista != vista {
            self.entrada = Entrada::OtraVista;
        } else if cambia && self.animaciones {
            self.cambio = Some(Instant::now());
        }
        if cambia && self.animaciones && self.estado.clase == Clase::Asistente(Fase::Error) {
            self.sacudida = Some(Instant::now());
        }
    }

    /// Esta actividad ocupa el sitio de otra que la rueda acaba de pasar: es
    /// la misma isla con otro contenido, no una nueva. Hereda cómo iba la
    /// entrada de la anterior (si ya había llegado, sigue llegada) y se queda
    /// con ella para el [`Relevo`]. Sin esto nacía otra vez del 60 % y
    /// transparente. Una que se estaba yendo no se hereda: esa sí es el relevo
    /// de una que terminó.
    // añadido en la 0.2.3
    // modificado en la 0.2.3 -- se queda la anterior para desplazar el contenido
    pub fn continuar_de(&mut self, mut anterior: Actividad) {
        if anterior.saliendo.is_some() {
            return;
        }
        self.desde = anterior.desde;
        self.entrada = anterior.entrada;
        // Sale de donde está ahora, aunque fuera a mitad de otro relevo: si
        // se pasan varias seguidas, la caja no vuelve atrás de golpe.
        let (tam0, radio0) = (anterior.tam_visible(), anterior.radio_visible());
        anterior.relevo = None;
        if self.animaciones && !tema::efectos_reducidos() {
            self.relevo = Some(Relevo {
                previa: Some(Box::new(anterior)),
                desde: Instant::now(),
                tam0,
                radio0,
                extender: false,
            });
        }
    }

    /// Lo que va del relevo, en la curva del muelle de la isla. `None` sin
    /// relevo o terminado.
    // añadido en la 0.2.3
    fn avance_relevo(&self) -> Option<(&Relevo, f32)> {
        let r = self.relevo.as_ref()?;
        let t = tema::fraccion(r.desde.elapsed(), tema::D_ISLA);
        (t < 1.0).then(|| (r, tema::C_MUELLE_ISLA.eval(t)))
    }

    /// Si hay que repintar el lienzo: el relevo mueve contenido, no solo la
    /// textura, así que no basta con lo que hace el compositor.
    // añadido en la 0.2.3
    pub fn animando_relevo(&self) -> bool {
        self.relevo.is_some()
    }

    /// Suelta la anterior cuando el relevo ha terminado. `true` si cambia el
    /// tamaño del lienzo (vuelve al de esta actividad sola).
    // añadido en la 0.2.3
    pub fn terminar_relevo(&mut self) -> bool {
        if self.relevo.is_some() && self.avance_relevo().is_none() {
            self.relevo = None;
            return true;
        }
        false
    }

    /// El tamaño de la tarjeta tal como se ve ahora, sin el margen de sombra.
    // añadido en la 0.2.3
    fn tam_visible(&self) -> (f32, f32) {
        let (w, h) = self.size_propio();
        let fin = (w - MARGEN_SOMBRA * 2.0, h - MARGEN_SOMBRA * 2.0);
        match self.avance_relevo() {
            // Sin pasarse: el lienzo mide lo mayor de los dos y un rebote de
            // la caja se saldría de él.
            Some((r, p)) => {
                let p = p.clamp(0.0, 1.0);
                (
                    r.tam0.0 + (fin.0 - r.tam0.0) * p,
                    r.tam0.1 + (fin.1 - r.tam0.1) * p,
                )
            }
            None => fin,
        }
    }

    // añadido en la 0.2.3
    fn radio_propio(&self) -> f32 {
        match self.vista {
            Vista::Compacta => RADIO_COMPACTO,
            _ if self.con_columna() => RADIO_ABIERTO_ISLA,
            _ => RADIO,
        }
    }

    // añadido en la 0.2.3
    fn radio_visible(&self) -> f32 {
        match self.avance_relevo() {
            Some((r, p)) => r.radio0 + (self.radio_propio() - r.radio0) * p.clamp(0.0, 1.0),
            None => self.radio_propio(),
        }
    }

    /// Empieza a irse. `false` sin animaciones: entonces se quita ya.
    // añadido en la 0.2.0
    pub fn salir(&mut self) -> bool {
        if self.animaciones && self.saliendo.is_none() {
            self.saliendo = Some(Instant::now());
        }
        self.saliendo.is_some()
    }

    // añadido en la 0.2.0
    pub fn salida_terminada(&self) -> bool {
        self.saliendo
            .is_some_and(|t| t.elapsed() >= tema::D_POPOVER)
    }

    /// Lo que hace con Zenith un clic fuera de la isla o Esc: quitarlo de en
    /// medio. Un clic fuera mientras escucha o piensa solo lo aparta a la
    /// burbuja, para seguir hablándole o no tirar una búsqueda por un clic
    /// distraído; Esc sí cancela.
    // añadido en la 0.2.0
    // modificado en la 0.2.0 -- escuchando, el clic fuera minimiza
    pub fn despedir_asistente(&mut self, cancelar: bool) -> Option<Accion> {
        let Clase::Asistente(fase) = self.estado.clase else {
            return None;
        };
        if self.saliendo.is_some() {
            return None;
        }
        let nombre = match fase {
            Fase::Escuchando | Fase::Pensando if cancelar => "cancel",
            Fase::Escuchando | Fase::Pensando => "minimize",
            Fase::Respuesta | Fase::Error | Fase::Hecho => "dismiss",
            Fase::Minimizado => return None,
        };
        Some(self.accion(nombre, String::new()))
    }

    /// El asistente no se abre con un clic: lo abre su fase. Una respuesta que
    /// hay que leer despliega la tarjeta y lo demás la recoge.
    // añadido en la 0.2.0
    // modificado en la 0.2.0 -- la paleta solo se relee al cambiar de fase
    fn seguir_fase(&mut self, fase_nueva: bool) {
        let Clase::Asistente(fase) = self.estado.clase else {
            return;
        };
        // Mientras escucha, Zenith publica el nivel de voz diez veces por
        // segundo; releer y parsear palette.json en cada una no cambia nada.
        if fase_nueva {
            self.tonos =
                tema::roles_paleta(["blue", "accent-2", "accent-3"]).unwrap_or([tema::acento(); 3]);
            self.sobre_tono =
                tema::roles_paleta(["on-accent"]).map_or_else(tema::sobre_acento, |[c]| c);
        }
        if fase.abierta() {
            self.cuerpo_asistente = crate::escritorio::lineas(
                &self.estado.subtitulo,
                ABIERTO_W - PAD * 2.0,
                tema::T_CUERPO,
                ASIS_CUERPO_MAX,
            )
            .join("\n");
        }
        let vista = if fase.abierta() {
            Vista::Abierta
        } else {
            Vista::Compacta
        };
        if vista != self.vista {
            self.vista = vista;
            self.desde = Instant::now();
        }
    }

    /// Recoge la tarjeta abierta. Es lo que hace un clic fuera de ella.
    // 0.1.0
    pub fn recoger(&mut self) -> bool {
        if self.vista == Vista::Compacta {
            return false;
        }
        self.colapsar();
        true
    }

    // 0.1.0
    pub fn desplegada(&self) -> bool {
        self.vista != Vista::Compacta
    }

    /// Título y subtítulo tal como los publicó la app, para el lector de
    /// pantalla.
    // añadido en la 0.2.2
    pub fn rotulos(&self) -> (&str, &str) {
        (&self.estado.titulo, &self.estado.subtitulo)
    }

    // 0.1.0
    pub fn app_id(&self) -> &str {
        &self.estado.app_id
    }
    // 0.1.0
    pub fn clase(&self) -> Clase {
        self.estado.clase
    }
    // 0.1.0
    pub fn set_animaciones(&mut self, animaciones: bool) {
        self.animaciones = animaciones;
    }

    /// Abre la tarjeta sin fingir una pulsación. Solo lo usa el contrato de
    /// previsualización del compositor en builds de desarrollo.
    // 0.1.0
    pub fn abrir_previsualizacion(&mut self) -> bool {
        if self.vista != Vista::Compacta {
            return false;
        }
        self.cambiar_vista(Vista::Abierta);
        true
    }

    /// Abre la tarjeta en su sitio, como continuación de otra que ya estaba
    /// abierta: pasar de una actividad a otra con la rueda creaba la nueva
    /// recogida y la isla se cerraba en medio del gesto. El asistente no: su
    /// vista la manda la fase.
    // añadido en la 0.2.2
    pub fn seguir_desplegada(&mut self) {
        if matches!(self.estado.clase, Clase::Asistente(_)) || self.vista != Vista::Compacta {
            return;
        }
        self.vista = Vista::Abierta;
        // Con relevo, abrirse lo cuenta la caja que crece: el encogido de
        // `OtraVista` encima era el tirón que se veía al pasar.
        if self.relevo.is_none() {
            self.entrada = Entrada::OtraVista;
        }
    }

    /// Abre la tarjeta compacta o recoge la abierta, con la misma transición
    /// que al pulsarla.
    // 0.1.0
    pub fn alternar_vista(&mut self) -> bool {
        match self.vista {
            Vista::Compacta => self.abrir_previsualizacion(),
            Vista::Abierta | Vista::Cola | Vista::Menu => {
                self.colapsar();
                true
            }
        }
    }

    /// Tramo de la cola que se muestra en «A continuación»: empieza por la
    /// canción que está sonando (`actual`) y sigue con las que vienen después,
    /// hasta [`COLA_MAX`] filas. Si ninguna viene marcada como actual, empieza
    /// desde el principio.
    // añadido en la 0.2.4
    fn cola_visible(&self) -> &[ItemCola] {
        let inicio = self.estado.cola.iter().position(|i| i.actual).unwrap_or(0);
        let fin = (inicio + COLA_MAX).min(self.estado.cola.len());
        &self.estado.cola[inicio..fin]
    }

    // 0.1.0
    fn filas_cola(&self) -> usize {
        self.cola_visible().len()
    }

    /// Alto del panel de la cola colgando del reproductor, incluida su
    /// separación. Cero si no hay cola que enseñar.
    // 0.1.0
    fn alto_cola(&self) -> f32 {
        let n = self.filas_cola();
        if n == 0 {
            return 0.0;
        }
        // El último PAD es el margen inferior de la tarjeta: la cola va
        // metida dentro, no pegada a su borde.
        COLA_SEP + COLA_PAD * 2.0 + COLA_CAB_H + COLA_FILA_H * n as f32 + PAD
    }

    // 0.1.0
    pub fn poner_burbujas(&mut self, burbujas: Vec<Burbuja>) {
        self.burbujas = burbujas;
    }

    /// Cuántas vivas hay además de la que se ve. Zenith no lleva contador:
    /// su píldora es su propia conversación.
    // añadido en la 0.2.0
    fn otras(&self) -> usize {
        if matches!(self.estado.clase, Clase::Asistente(_)) {
            return 0;
        }
        self.burbujas.len().saturating_sub(1)
    }

    // 0.1.0
    // modified 0.2.0 -- crece lo del «+N» si hay más vivas
    fn ancho_pildora(&self) -> f32 {
        let base = match self.estado.clase {
            Clase::Player => PLAYER_COMPACTO_W,
            Clase::Timer => TIMER_COMPACTO_W,
            Clase::Recorder => RECORDER_COMPACTO_W,
            Clase::Transfer => TRANSFER_COMPACTO_W,
            Clase::Calendar => CALENDARIO_COMPACTO_W,
            Clase::Monitor => MONITOR_COMPACTO_W,
            Clase::Auriculares => AURICULARES_COMPACTO_W,
            Clase::Asistente(Fase::Minimizado) => ASIS_MINI,
            Clase::Asistente(_) => ASIS_COMPACTO_W,
        };
        base + if self.otras() > 0 { CONTADOR_W } else { 0.0 }
    }

    // añadido en la 0.2.0
    fn alto_pildora(&self) -> f32 {
        match self.estado.clase {
            Clase::Asistente(Fase::Minimizado) => ASIS_MINI,
            Clase::Asistente(_) => ASIS_COMPACTO_H,
            _ => PLAYER_COMPACTO_H,
        }
    }

    /// Si la tarjeta abierta lleva la columna de actividades: con más de una
    /// viva. Zenith no, que tiene su propia tarjeta y se despide solo.
    // añadido en la 0.2.0
    fn con_columna(&self) -> bool {
        self.vista != Vista::Compacta
            && self.burbujas.len() > 1
            && !matches!(self.estado.clase, Clase::Asistente(_))
    }

    /// Centro vertical y lado de cada icono de la columna, en coordenadas de la
    /// tarjeta. Centrada en el alto de la tarjeta.
    // añadido en la 0.2.0
    fn columna(&self) -> Vec<(f32, f32)> {
        let (_, h) = self.size_propio();
        let h = h - MARGEN_SOMBRA * 2.0;
        let lados: Vec<f32> = self
            .burbujas
            .iter()
            .map(|b| {
                if b.actual {
                    COLUMNA_ACTUAL
                } else {
                    COLUMNA_OTRA
                }
            })
            .collect();
        let total: f32 =
            lados.iter().sum::<f32>() + COLUMNA_SEP * (lados.len().saturating_sub(1)) as f32;
        let mut y = (h - total) / 2.0;
        lados
            .into_iter()
            .map(|l| {
                let c = (y + l / 2.0, l);
                y += l + COLUMNA_SEP;
                c
            })
            .collect()
    }

    /// La parte del buffer que se ve y recibe clics, como `(x, ancho)` en
    /// coordenadas de la tarjeta. El hueco simétrico de la izquierda no es de
    /// nadie: si el compositor lo contara, se tragaría clics del panel.
    // 0.1.0
    // modificado en la 0.2.3 -- durante un relevo, la caja centrada del lienzo
    pub fn zona(&self) -> (f32, f32) {
        let (w, _) = self.size();
        let (visible, _) = self.tam_visible();
        ((w - MARGEN_SOMBRA * 2.0 - visible) / 2.0, visible)
    }

    /// El `app_id` del icono de la columna bajo el punto, en coordenadas del
    /// buffer. Toda la anchura de la columna cuenta, no solo el círculo: los de
    /// 18 px son pequeños para acertarles.
    // 0.1.0
    // modified 0.2.0 -- de las burbujas junto a la píldora a la columna de la abierta
    pub fn burbuja_en(&self, x: f32, y: f32) -> Option<&str> {
        if !self.con_columna() {
            return None;
        }
        let (x, y) = (x - MARGEN_SOMBRA, y - MARGEN_SOMBRA);
        if x < ABIERTO_W || x > ABIERTO_W + COLUMNA {
            return None;
        }
        self.columna()
            .iter()
            .zip(&self.burbujas)
            .find(|((c, l), _)| (y - c).abs() <= l / 2.0 + COLUMNA_SEP / 2.0)
            .map(|(_, b)| b.app_id.as_str())
    }

    /// El lienzo: mientras dura un relevo, lo bastante grande para la caja
    /// de partida y la de llegada, para no rehacerlo en cada fotograma.
    // 0.1.0
    // modificado en la 0.2.3 -- cabe el relevo
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.size_propio();
        match &self.relevo {
            Some(r) => (
                w.max(r.tam0.0 + MARGEN_SOMBRA * 2.0),
                h.max(r.tam0.1 + MARGEN_SOMBRA * 2.0),
            ),
            None => (w, h),
        }
    }

    /// Lo que mide esta actividad sola, con el margen de sombra.
    // 0.1.0
    fn size_propio(&self) -> (f32, f32) {
        let (w, h) = match self.vista {
            Vista::Compacta => (self.ancho_pildora(), self.alto_pildora()),
            Vista::Menu => (
                ABIERTO_W + if self.con_columna() { COLUMNA } else { 0.0 },
                PAD + CAB_EXTRA + MENU_FILA_H * MENU_FILAS as f32 + PAD,
            ),
            Vista::Abierta | Vista::Cola => (
                ABIERTO_W + if self.con_columna() { COLUMNA } else { 0.0 },
                // Zenith no lleva cabecera: su tarjeta es la conversación.
                if matches!(self.estado.clase, Clase::Asistente(_)) {
                    0.0
                } else {
                    CAB_EXTRA
                } + match self.estado.clase {
                    Clase::Player if self.vista == Vista::Cola => PLAYER_H + self.alto_cola(),
                    Clase::Player => PLAYER_H,
                    Clase::Timer => TIMER_H,
                    Clase::Recorder => RECORDER_H,
                    Clase::Transfer => TRANSFER_H,
                    Clase::Calendar => CAL_H,
                    Clase::Auriculares => AUR_H,
                    Clase::Asistente(_) => self.boton_asistente_y() + BOTON_H + PAD,
                    Clase::Monitor => {
                        PAD * 2.0
                            + ROTULO_H
                            + 8.0
                            + MON_FILA_H * self.estado.metricas.len().max(1) as f32
                    }
                },
            ),
        };
        (w + MARGEN_SOMBRA * 2.0, h + MARGEN_SOMBRA * 2.0)
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.animaciones
            && (self.desde.elapsed() < tema::D_ISLA
                || self.saliendo.is_some()
                || self.cambio.is_some_and(|t| t.elapsed() < tema::D_ISLA)
                || self.relevo.is_some()
                || self
                    .sacudida
                    .is_some_and(|t| t.elapsed() < tema::D_ISLA + D_SACUDIDA)
                || (self.vista == Vista::Compacta && self.sonando() && self.con_ondas()))
    }

    /// La voz que oye Zenith, siguiendo al último nivel publicado sin saltos.
    /// En forma cerrada, para leerla desde `&self` en cada fotograma.
    // añadido en la 0.2.0
    pub fn nivel_suave(&self) -> f32 {
        let objetivo = if self.sonando() {
            self.estado.nivel.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if !self.animaciones {
            return objetivo;
        }
        let (desde, t) = self.nivel_desde;
        objetivo + (desde - objetivo) * (-t.elapsed().as_secs_f32() / NIVEL_TAU).exp()
    }

    /// Las clases cuya píldora lleva una onda viva.
    // añadido en la 0.2.0
    fn con_ondas(&self) -> bool {
        matches!(
            self.estado.clase,
            Clase::Player
                | Clase::Recorder
                | Clase::Asistente(Fase::Escuchando | Fase::Pensando | Fase::Minimizado)
        )
    }

    /// Si toca reconstruir el bitmap por el siguiente paso del vúmetro.
    /// La animación de entrada no entra aquí: alfa, escala y desplazamiento se
    /// aplican a la textura ya pintada desde el compositor/GPU.
    // 0.1.0
    pub fn ondas_pendientes(&self) -> bool {
        self.animaciones
            && self.vista == Vista::Compacta
            && self.sonando()
            && self.con_ondas()
            && self.ultimo_frame_ondas.elapsed()
                >= if matches!(self.estado.clase, Clase::Asistente(_)) {
                    FRAME_ASISTENTE
                } else {
                    FRAME_ONDAS
                }
    }

    // 0.1.0
    pub fn marcar_ondas_pintadas(&mut self) {
        self.ultimo_frame_ondas = Instant::now();
    }

    /// Alfa, zoom y desplazamiento `(x, y)` con que se compone la isla.
    // 0.1.0
    // modificado en la 0.2.0 -- desplazamiento horizontal para la sacudida del error
    pub fn entrada(&self) -> (f32, f32, f32, f32) {
        // Los efectos reducidos van por el mismo camino que apagar las
        // animaciones de la isla: sin zoom, sin sacudida y sin respirar con la voz.
        if !self.animaciones || tema::efectos_reducidos() {
            return (1.0, 1.0, 0.0, 0.0);
        }
        let p = tema::C_MUELLE_ISLA.eval(tema::fraccion(self.desde.elapsed(), tema::D_ISLA));
        let a = tema::C_ENTRADA.eval(tema::fraccion(self.desde.elapsed(), tema::D_TARJETA));
        // Crece desde la barra: el compositor escala desde el centro de la
        // superficie, así que se sube lo que el zoom baja el canto superior de
        // la tarjeta y el ancla queda arriba, donde está la barra.
        let (zoom0, alfa0) = match self.entrada {
            Entrada::Nueva => (0.6, 0.0),
            // Píldora y tarjeta de Zenith miden lo mismo de ancho: un 8 % de
            // encogido basta para que se lea como «se despliega».
            Entrada::OtraVista => (0.92, 0.5),
        };
        let mut zoom = zoom0 + (1.0 - zoom0) * p;
        let mut alfa = alfa0 + (1.0 - alfa0) * a;
        if let Some(t) = self.cambio {
            let c = tema::C_MUELLE_ISLA.eval(tema::fraccion(t.elapsed(), tema::D_ISLA));
            zoom *= 0.94 + 0.06 * c;
        }
        // added 0.2.3 -- empujón al pasar de actividad, como `paso` de la maqueta
        let mut dy_paso = 0.0;
        if let Some(r) = self.relevo.as_ref().filter(|r| r.previa.is_some()) {
            let f = tema::fraccion(r.desde.elapsed(), PASO);
            if f < 1.0 {
                let b = tema::C_SUAVE.eval(if f < 0.4 { f / 0.4 } else { (1.0 - f) / 0.6 });
                zoom *= 1.0 + 0.03 * b;
                dy_paso = 2.0 * b;
            }
        }
        // Se va hacia la barra de la que salió, más deprisa de lo que entra.
        if let Some(t) = self.saliendo {
            let s = tema::C_SUAVE.eval(tema::fraccion(t.elapsed(), tema::D_POPOVER));
            alfa *= 1.0 - s;
            zoom *= 1.0 - 0.15 * s;
        }
        // Respira con la voz mientras escucha: es lo que dice «te oigo» sin
        // tener que mirar las barras. Lo hace la GPU, no repinta nada.
        if matches!(
            self.estado.clase,
            Clase::Asistente(Fase::Escuchando | Fase::Minimizado)
        ) {
            zoom *= 1.0 + HINCHADO_VOZ * self.nivel_suave();
        }
        // Tras aterrizar la tarjeta, no a la vez: crecer y sacudirse en el
        // mismo gesto se leía como un tirón, no como un «no».
        let dx = self.sacudida.map_or(0.0, |t| {
            let f = tema::fraccion(t.elapsed().saturating_sub(tema::D_ISLA), D_SACUDIDA);
            SACUDIDA_PX * (1.0 - f) * (f * 2.5 * std::f32::consts::TAU).sin()
        });
        let (_, h) = self.size();
        (alfa, zoom, dx, -(h / 2.0 - MARGEN_SOMBRA) * (1.0 - zoom) + dy_paso)
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let x = x - MARGEN_SOMBRA;
        let y = y - MARGEN_SOMBRA;
        // Durante el relevo lo que hay bajo el dedo se está moviendo: son
        // 420 ms, y un clic ahí caería en la tarjeta que se va.
        // Un cambio de vista no mueve el contenido: ese sí atiende.
        let moviendose = self
            .avance_relevo()
            .is_some_and(|(r, _)| r.previa.is_some());
        if x < 0.0 || y < 0.0 || self.saliendo.is_some() || moviendose {
            return None;
        }
        if let Clase::Asistente(fase) = self.estado.clase {
            return self.pulsar_asistente(fase, x, y);
        }
        if self.vista == Vista::Compacta {
            self.cambiar_vista(Vista::Abierta);
            return None;
        }
        if x >= ABIERTO_W {
            return None;
        }
        if y < PAD + CAB_EXTRA - 4.0 {
            if x < CAB_APP_X + self.ancho_cab_app() {
                return Some(self.accion(ISLA_ABRIR, String::new()));
            }
            if x > ABIERTO_W - MAS_DER - MAS_W - 6.0 {
                self.alternar_menu();
                return None;
            }
            self.colapsar();
            return None;
        }
        if self.vista == Vista::Menu {
            let fila = ((y - PAD - CAB_EXTRA) / MENU_FILA_H).floor();
            let nombre = match fila as i32 {
                0 => ISLA_ABRIR,
                1 if !self.es_la_primera() => ISLA_PRIMERO,
                2 => ISLA_OCULTAR,
                3 => ISLA_AJUSTES,
                _ => return None,
            };
            self.cambiar_vista(Vista::Abierta);
            return Some(self.accion(nombre, String::new()));
        }
        let y = y - CAB_EXTRA;
        match self.estado.clase {
            Clase::Player => self.pulsar_player(x, y),
            Clase::Timer => {
                // La cabecera entera cierra: con una tarjeta sin chevron, es lo
                // que queda por descubrir y no hay nada más ahí que pulsar.
                if y < TIMER_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                let nombre = if x < ABIERTO_W / 2.0 {
                    if self.estado.restante_ms < 0 {
                        "snooze"
                    } else if self.estado.pausado {
                        "resume"
                    } else {
                        "pause"
                    }
                } else {
                    "stop"
                };
                Some(self.accion(nombre, String::new()))
            }
            Clase::Transfer => {
                if y < TRANSFER_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                Some(self.accion("cancel", String::new()))
            }
            Clase::Calendar => {
                if y < CAL_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                Some(self.accion("dismiss", String::new()))
            }
            Clase::Auriculares => {
                if y < AUR_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                let Some(actual) = self.estado.anc else {
                    return Some(self.accion("open", String::new()));
                };
                let tercio = ((x - PAD) / ((ABIERTO_W - PAD * 2.0) / 3.0)).floor();
                let modo = *MODOS_RUIDO.get(tercio.clamp(0.0, 2.0) as usize)?;
                if modo == actual {
                    return None;
                }
                // Se marca ya: los Buds tardan en confirmar y el segmentado
                // tiene que responder al dedo.
                self.estado.anc = Some(modo);
                Some(self.accion("anc", modo.to_string()))
            }
            // Solo informa: cualquier clic lo vuelve a la píldora.
            Clase::Monitor => {
                self.colapsar();
                None
            }
            Clase::Asistente(_) => None,
            Clase::Recorder => {
                if y < REC_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                // Pausa a la izquierda y detener a la derecha, en las dos
                // mitades de la tarjeta: los botones van centrados a cada lado
                // del eje, así que cada mitad contiene el suyo entero.
                let nombre = if x >= ABIERTO_W / 2.0 {
                    "stop"
                } else if self.estado.pausado {
                    "resume"
                } else {
                    "pause"
                };
                Some(self.accion(nombre, String::new()))
            }
        }
    }

    fn alto_cuerpo_asistente(&self) -> f32 {
        self.cuerpo_asistente.lines().count() as f32 * ASIS_LINEA_H
    }

    fn boton_asistente_y(&self) -> f32 {
        PAD + ASIS_MARCA_H
            + 16.0
            + ASIS_TITULO_H
            + if self.cuerpo_asistente.is_empty() {
                0.0
            } else {
                8.0 + self.alto_cuerpo_asistente()
            }
            + 20.0
    }

    fn pulsar_asistente(&mut self, fase: Fase, x: f32, y: f32) -> Option<Accion> {
        let (w, h) = self.size_propio();
        if !(0.0..w - MARGEN_SOMBRA * 2.0).contains(&x)
            || !(0.0..h - MARGEN_SOMBRA * 2.0).contains(&y)
        {
            return None;
        }
        if self.vista != Vista::Compacta {
            let cerrar_x = ABIERTO_W - PAD - ASIS_CERRAR;
            if (cerrar_x..cerrar_x + ASIS_CERRAR).contains(&x)
                && (PAD..PAD + ASIS_CERRAR).contains(&y)
            {
                return Some(self.accion("dismiss", String::new()));
            }
            if (PAD..ABIERTO_W - PAD).contains(&x)
                && (self.boton_asistente_y()..self.boton_asistente_y() + BOTON_H).contains(&y)
            {
                return Some(self.accion(
                    if fase == Fase::Error {
                        "retry"
                    } else {
                        "dismiss"
                    },
                    String::new(),
                ));
            }
            return None;
        }
        let cerrar = (ASIS_COMPACTO_W - ASIS_LADO - ASIS_CERRAR_PILDORA
            ..ASIS_COMPACTO_W - ASIS_LADO)
            .contains(&x)
            && ((ASIS_COMPACTO_H - ASIS_CERRAR_PILDORA) / 2.0
                ..(ASIS_COMPACTO_H + ASIS_CERRAR_PILDORA) / 2.0)
                .contains(&y);
        match fase {
            Fase::Minimizado => Some(self.accion("expand", String::new())),
            Fase::Escuchando | Fase::Pensando if cerrar => {
                Some(self.accion("cancel", String::new()))
            }
            Fase::Respuesta | Fase::Error if cerrar => Some(self.accion("dismiss", String::new())),
            Fase::Respuesta | Fase::Error => {
                self.vista = Vista::Abierta;
                self.desde = Instant::now();
                None
            }
            // La píldora entera responde: mientras escucha, pulsarla es «ya
            // he terminado de hablar»; mientras piensa, «sigue, pero aparte».
            Fase::Escuchando => Some(self.accion("finish", String::new())),
            Fase::Pensando => Some(self.accion("minimize", String::new())),
            Fase::Hecho => Some(self.accion("dismiss", String::new())),
        }
    }

    // 0.1.0
    /// Abre o cierra el menú ⋯. Es lo que hacen el botón y el clic derecho.
    // añadido en la 0.2.2
    pub fn alternar_menu(&mut self) -> bool {
        if matches!(self.estado.clase, Clase::Asistente(_)) || self.saliendo.is_some() {
            return false;
        }
        self.cambiar_vista(if self.vista == Vista::Menu {
            Vista::Abierta
        } else {
            Vista::Menu
        });
        true
    }

    /// Si se puede recoger sola al apartar el puntero: con el menú abierto se
    /// está eligiendo algo, y Zenith se despide por su cuenta.
    // añadido en la 0.2.2
    pub fn recogible(&self) -> bool {
        matches!(self.vista, Vista::Abierta | Vista::Cola)
            && !matches!(self.estado.clase, Clase::Asistente(_))
    }

    /// La que se ve ya es la primera del orden de la isla.
    // añadido en la 0.2.2
    fn es_la_primera(&self) -> bool {
        self.burbujas.first().is_none_or(|b| b.actual)
    }

    /// Ancho de «icono + nombre + ›» con su relleno, el mismo que se pinta.
    // añadido en la 0.2.2
    fn ancho_cab_app(&self) -> f32 {
        4.0 + CAB_ICONO
            + 7.0
            + crate::widget::ancho_con_fuente(&self.app_nombre, 12.0, negrita())
            + 7.0
            + 12.0
            + 10.0
    }

    // modificado en la 0.2.3 -- con cambiar_vista
    fn colapsar(&mut self) {
        self.cambiar_vista(Vista::Compacta);
    }

    /// Otra vista de la misma actividad: la caja se estira o encoge desde
    /// donde está con el muelle de la isla. Antes se reiniciaba `desde` con
    /// `entrada` en `Nueva`, y abrir el menú ⋯ o la cola la hacía nacer otra
    /// vez del 60 % y transparente, como si llegara una actividad nueva.
    // añadido en la 0.2.3
    fn cambiar_vista(&mut self, vista: Vista) {
        if self.vista == vista {
            return;
        }
        let antes = self.vista;
        let (tam0, radio0) = (self.tam_visible(), self.radio_visible());
        self.vista = vista;
        let extender = matches!(
            (antes, vista),
            (Vista::Abierta, Vista::Cola) | (Vista::Cola, Vista::Abierta)
        );
        if self.animaciones && !tema::efectos_reducidos() {
            self.relevo = Some(Relevo {
                previa: None,
                desde: Instant::now(),
                tam0,
                radio0,
                extender,
            });
        }
    }

    // 0.1.0
    fn pulsar_player(&mut self, x: f32, y: f32) -> Option<Accion> {
        if y < CTRL_Y {
            self.colapsar();
            return None;
        }
        if y < CTRL_Y + CTRL_H {
            // Los cinco controles, medidos desde el centro hacia fuera con los
            // mismos anchos y separaciones con que se dibujan.
            let centro = ABIERTO_W / 2.0;
            let d = (x - centro).abs();
            let mitad_play = CTRL_PLAY / 2.0;
            let fin_lado = mitad_play + CTRL_SEP + CTRL_LADO;
            let fin_plano = fin_lado + CTRL_SEP + CTRL_PLANO;
            let nombre = if d <= mitad_play {
                "play-pause"
            } else if d <= fin_lado {
                if x < centro { "previous" } else { "next" }
            } else if d <= fin_plano {
                if x < centro {
                    "shuffle"
                } else {
                    // Favorita la que suena, como la maqueta. Repetir sigue en
                    // la app y en el centro de control.
                    let actual = self.estado.cola.iter_mut().find(|i| i.actual)?;
                    actual.favorita = !actual.favorita;
                    let id = actual.id.clone();
                    return Some(self.accion("favorite", id));
                }
            } else {
                return None;
            };
            // El icono cambia ya, sin esperar a que el reproductor conteste:
            // la ida y vuelta por D-Bus más su publicación agrupada se notaba
            // como un botón que no responde. Si la orden no llega, el
            // siguiente latido del reproductor (1 s) deja el estado real.
            if nombre == "play-pause" {
                self.estado.pausado = !self.estado.pausado;
                self.estado.activo = !self.estado.pausado;
            }
            return Some(self.accion(nombre, String::new()));
        }
        // La frontera con la fila de abajo va en el divisor, a mitad del hueco.
        if y < SEC_Y - 11.0 {
            let ini = PAD + TIEMPO_W + 10.0;
            let fin = ABIERTO_W - PAD - TIEMPO_W - 10.0;
            let value = ((x - ini) / (fin - ini)).clamp(0.0, 1.0);
            return Some(self.accion("seek", format!("{value:.4}")));
        }
        if y < PLAYER_H - PAD / 2.0 {
            if x < ABIERTO_W / 2.0 {
                if self.filas_cola() == 0 {
                    return None;
                }
                self.cambiar_vista(if self.vista == Vista::Cola {
                    Vista::Abierta
                } else {
                    Vista::Cola
                });
                return None;
            }
            // El icono del altavoz no es parte de la pista: un clic en él
            // que bajase el volumen a 0 sería una sorpresa.
            if x < VOL_X0 - 6.0 {
                return None;
            }
            let recorrido = VOL_X1 - VOL_X0 - VOL_PULGAR;
            let f = ((x - VOL_X0 - VOL_PULGAR / 2.0) / recorrido).clamp(0.0, 1.0);
            let value = (f * 100.0).round() as u8;
            // Igual que reproducir: el pulgar va ya donde se pulsó y el
            // siguiente latido del reproductor confirma el valor real.
            self.estado.volumen = value;
            return Some(self.accion("volume", value.to_string()));
        }
        // Por debajo del reproductor está el panel de la cola.
        let y = y - (PLAYER_H + COLA_SEP + COLA_PAD + COLA_CAB_H);
        if y < 0.0 {
            return None;
        }
        let i = (y / COLA_FILA_H) as usize;
        let item = self.cola_visible().get(i)?;
        let id = item.id.clone();
        let nombre = if x > ABIERTO_W - COLA_PAD - 40.0 {
            "favorite"
        } else {
            "queue-goto"
        };
        Some(self.accion(nombre, id))
    }

    // 0.1.0
    fn accion(&self, raw: &str, fallback: String) -> Accion {
        // Los auriculares no son una app con ventana: «abrir» es pedirle a
        // bookos-budsd que abra su página de Ajustes, que es lo que ya atiende.
        let raw = if raw == ISLA_ABRIR && self.estado.clase == Clase::Auriculares {
            "open"
        } else {
            raw
        };
        let (nombre, valor) = raw
            .split_once('\u{1f}')
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .unwrap_or_else(|| (raw.to_string(), fallback));
        Accion {
            app_id: self.estado.app_id.clone(),
            nombre,
            valor,
        }
    }

    // ── Vista ────────────────────────────────────────────────────────────

    // 0.1.0
    // modificado en la 0.2.3 -- el relevo desplaza el contenido
    pub fn view(&self) -> PanelElement<'_> {
        let (w, h) = self.size();
        if let Some((r, _)) = self.avance_relevo() {
            return self.view_relevo(r, w, h);
        }
        container(self.tarjeta())
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .padding(MARGEN_SOMBRA)
            .into()
    }

    // añadido en la 0.2.3
    fn tarjeta(&self) -> PanelElement<'_> {
        match self.vista {
            Vista::Compacta => self.compacta(),
            _ if self.con_columna() => self.con_su_columna(),
            _ => self.abierta(),
        }
    }

    /// Una caja con el tamaño y el radio de este momento, y dentro la tarjeta
    /// que se va o la que llega, sin fondo y recortadas por ella. Ninguna se
    /// mueve: se tapan con un velo del color de la isla, que es como iced
    /// puede fundir un árbol de widgets (no hay opacidad por capa).
    // añadido en la 0.2.3
    // modified 0.2.3 -- velos temporizados como la maqueta, sin desplazar
    fn view_relevo<'a>(&'a self, r: &'a Relevo, w: f32, h: f32) -> PanelElement<'a> {
        let (cw, ch) = self.tam_visible();
        let radio = self.radio_visible();
        let pasado = r.desde.elapsed();
        let colocar = |tarjeta: PanelElement<'a>| -> PanelElement<'a> {
            crate::recorte::Recorte::new(
                container(tarjeta)
                    .width(Length::Fixed(cw))
                    .align_x(Horizontal::Center),
                cw,
                ch,
                0.0,
            )
            .into()
        };
        // La tarjeta lleva su propio margen de sombra fuera: `tarjeta()` es
        // solo la caja, así que entra tal cual.
        SIN_FONDO.with(|c| c.set(true));
        let sale = tema::C_SUAVE.eval(tema::fraccion(pasado, RELEVO_SALE));
        let entra = tema::C_SUAVE.eval(tema::fraccion(
            pasado.saturating_sub(RELEVO_ESPERA),
            RELEVO_ENTRA,
        ));
        let mut capas = stack![];
        // Con `previa` en `None` (otra vista de la misma) la vista vieja ya no
        // se puede construir; en la maqueta se apaga en 100 ms, así que lo
        // que se pierde es un destello.
        // Las dos nunca se ven a la vez (una acaba a los 100 ms y la otra
        // empieza a los 190), así que basta un velo.
        let mut velo = 0.0;
        if r.extender {
            // Al desplegar o recoger la cola, el reproductor superior se queda
            // intacto sin apagarse: `Recorte` descubre u oculta la cola al ritmo
            // del alto actual de la caja.
            capas = capas.push(colocar(self.tarjeta()));
        } else {
            if let Some(previa) = r.previa.as_ref().filter(|_| sale < 1.0) {
                capas = capas.push(colocar(previa.tarjeta()));
                velo = sale;
            }
            if entra > 0.0 {
                capas = capas.push(colocar(self.tarjeta()));
                velo = 1.0 - entra;
            }
        }
        SIN_FONDO.with(|c| c.set(false));
        let caja = container(capas)
            .width(Length::Fixed(cw))
            .height(Length::Fixed(ch))
            .style(move |_| container::Style {
                background: Some(isla().into()),
                border: Border {
                    radius: radio.into(),
                    ..Default::default()
                },
                // Sin sombra mientras se mueve: la de iced es un SDF por
                // píxel y repintarla en cada fotograma llevaba el relevo
                // abierto de 3–8 ms a ~55 ms (debug, example actividad).
                // La tarjeta en reposo la vuelve a llevar.
                ..Default::default()
            });
        // Encima de la caja y fuera del `Recorte`: dentro de su capa el
        // relleno translúcido pasaba por la máscara del recorte y la tarjeta
        // abierta costaba 18–43 ms por fotograma; aquí, ver la medida en el
        // ejemplo `actividad`. A 0 o a 1 no se pinta: el alfa 0 también se
        // rasteriza.
        let caja: PanelElement<'a> = if velo > 0.0 && velo < 1.0 {
            stack![
                caja,
                container(Space::new())
                    .width(Length::Fixed(cw))
                    .height(Length::Fixed(ch))
                    .style(move |_| container::Style {
                        background: Some(tema::alfa(isla(), velo).into()),
                        border: Border {
                            radius: radio.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            ]
            .into()
        } else {
            caja.into()
        };
        container(caja)
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .padding(MARGEN_SOMBRA)
            .align_x(Horizontal::Center)
            .into()
    }

    /// La tarjeta abierta y, a su derecha, la columna con un icono por actividad
    /// viva: la actual grande y a todo color, las demás pequeñas y apagadas.
    /// Las dos sobre el mismo negro: la junta no se ve.
    // añadido en la 0.2.0
    fn con_su_columna(&self) -> PanelElement<'_> {
        let (_, h) = self.size_propio();
        let h = h - MARGEN_SOMBRA * 2.0;
        let mut col = column![];
        let mut y = 0.0;
        for ((c, lado), b) in self.columna().into_iter().zip(&self.burbujas) {
            col = col
                .push(Space::new().height(Length::Fixed(c - lado / 2.0 - y)))
                .push(icono_columna(b, lado));
            y = c + lado / 2.0;
        }
        let con_fondo = !sin_fondo();
        container(row![
            self.abierta(),
            container(col)
                .width(Length::Fixed(COLUMNA))
                .height(Length::Fixed(h))
                .align_x(Horizontal::Center),
        ])
        .style(move |_| {
            if !con_fondo {
                return container::Style::default();
            }
            container::Style {
                background: Some(isla().into()),
                border: Border {
                    radius: RADIO_ABIERTO_ISLA.into(),
                    ..Default::default()
                },
                shadow: tema::sombra_popover(),
                ..Default::default()
            }
        })
        .into()
    }

    /// El color de lo que se pulsa: el acento del sistema, también en la
    /// música. La grabadora pinta su rojo aparte, que es estado y no acción.
    // 0.1.0
    fn color(&self) -> Color {
        match self.estado.clase {
            // Informativo: el acento (HIG: «azul = interactivo o activo») no
            // se gasta en decorar una lectura de reojo.
            Clase::Monitor | Clase::Auriculares => tema::TEXTO2,
            Clase::Asistente(_) => self.tonos[0],
            _ => tema::acento(),
        }
    }

    // 0.1.0
    fn sonando(&self) -> bool {
        self.estado.activo && !self.estado.pausado
    }

    // 0.1.0
    // modified 0.2.0 -- una actividad a la vez: sin burbujas al lado
    fn compacta(&self) -> PanelElement<'_> {
        self.pildora()
    }

    /// Las píldoras van todas igual: icono o portada, una identificación breve
    /// y el dato principal a la derecha, en un ancho fijo por clase para que
    /// cambiar una cifra no mueva nada.
    // 0.1.0
    fn pildora(&self) -> PanelElement<'_> {
        if let Clase::Asistente(fase) = self.estado.clase {
            return self.pildora_asistente(fase);
        }
        let color = self.color();
        let (contenido, ancho, lados): (PanelElement<'_>, f32, u16) = match self.estado.clase {
            Clase::Player => (
                row![
                    caratula(self.arte.as_ref(), 18.0),
                    Space::new().width(Length::Fill),
                    ondas(self.estado.nivel, self.sonando(), color, 14.0, 4, 2.5),
                ]
                .spacing(8)
                .align_y(Vertical::Center)
                .into(),
                PLAYER_COMPACTO_W,
                8,
            ),
            Clase::Timer => (
                row![
                    icono_teñido(icono_inline(RELOJ), 16.0, color),
                    text(formato_tiempo(self.estado.restante_ms))
                        .size(13)
                        .font(negrita())
                        .color(if self.estado.restante_ms < 0 {
                            tema::rojo()
                        } else {
                            tema::texto()
                        }),
                ]
                .spacing(8)
                .align_y(Vertical::Center)
                .into(),
                TIMER_COMPACTO_W,
                12,
            ),
            Clase::Transfer => (
                fila_compacta(
                    icono_teñido(icono_inline(DOCUMENTO), 16.0, tema::texto()),
                    &self.estado.titulo,
                    TRANSFER_COMPACTO_W - 40.0,
                    format!("{:.0} %", self.estado.nivel.clamp(0.0, 1.0) * 100.0),
                    tema::TEXTO2,
                ),
                TRANSFER_COMPACTO_W,
                12,
            ),
            Clase::Calendar => (
                fila_compacta(
                    icono_teñido(icono_inline(CALENDARIO), 16.0, tema::texto()),
                    &self.estado.titulo,
                    CALENDARIO_COMPACTO_W - 64.0,
                    cuenta_atras(self.estado.restante_ms),
                    tema::TEXTO2,
                ),
                CALENDARIO_COMPACTO_W,
                12,
            ),
            Clase::Monitor => (
                row![
                    icono_teñido(icono_inline(CHIP), 16.0, color),
                    text(crate::emergente::recortar_texto(
                        &resumen_metricas(&self.estado.metricas),
                        MONITOR_COMPACTO_W - 16.0 - 8.0 - 24.0,
                        11.0
                    ))
                    .size(11)
                    .font(negrita())
                    .wrapping(iced_core::text::Wrapping::None)
                    .width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Vertical::Center)
                .into(),
                MONITOR_COMPACTO_W,
                12,
            ),
            Clase::Auriculares => {
                // La más baja de las de los auriculares: es la que decide
                // cuánto queda de escucha. El estuche no cuenta, y un auricular
                // sin lectura tampoco: su 0 es «no lo sé», no «descargado».
                let peor = self
                    .estado
                    .metricas
                    .iter()
                    .take(2)
                    .filter(|m| !desconocida(m))
                    .min_by(|a, b| a.valor.total_cmp(&b.valor));
                (
                    fila_compacta(
                        image(FOTOS_BUDS[4].clone()).width(18.0).height(18.0).into(),
                        &self.estado.titulo,
                        AURICULARES_COMPACTO_W - 40.0,
                        peor.map_or_else(|| "—".to_string(), |m| m.texto.clone()),
                        if peor.is_some_and(|m| m.critica) {
                            tema::rojo()
                        } else {
                            tema::TEXTO2
                        },
                    ),
                    AURICULARES_COMPACTO_W,
                    12,
                )
            }
            // Lo dibuja `pildora_asistente`, arriba.
            Clase::Asistente(_) => (Space::new().into(), ASIS_COMPACTO_W, 12),
            Clase::Recorder => (
                row![
                    punto(self.sonando(), tema::rojo(), 8.0),
                    text(if self.estado.pausado {
                        "En pausa"
                    } else {
                        "Grabando"
                    })
                    .size(12)
                    .font(negrita())
                    .width(Length::Fill),
                    text(formato_tiempo(self.estado.posicion_ms))
                        .size(12)
                        .font(negrita()),
                ]
                .spacing(8)
                .align_y(Vertical::Center)
                .into(),
                RECORDER_COMPACTO_W,
                12,
            ),
        };
        // Se ve una, pero las demás siguen ahí: el «+N» lo dice sin quitarle
        // sitio a la principal, que conserva su ancho fijo.
        let (contenido, ancho) = match self.otras() {
            0 => (contenido, ancho),
            n => (
                row![
                    container(contenido).width(Length::Fill),
                    text(format!("+{n}"))
                        .size(11)
                        .font(negrita())
                        .color(tema::TEXTO2)
                        .width(Length::Fixed(CONTADOR_W - 8.0))
                        .align_x(Horizontal::Right),
                ]
                .spacing(8)
                .align_y(Vertical::Center)
                .into(),
                ancho + CONTADOR_W,
            ),
        };
        tarjeta(
            contenido,
            ancho,
            PLAYER_COMPACTO_H,
            RADIO_COMPACTO,
            [4, lados],
        )
    }

    // 0.1.0
    fn abierta(&self) -> PanelElement<'_> {
        if let Clase::Asistente(fase) = self.estado.clase {
            return self.abierta_asistente(fase);
        }
        let color = self.color();
        let (w, h) = self.size_propio();
        // La tarjeta en sí no incluye la columna: va al lado, en `view`.
        let columna = if self.con_columna() { COLUMNA } else { 0.0 };
        let (w, h) = (w - MARGEN_SOMBRA * 2.0 - columna, h - MARGEN_SOMBRA * 2.0);
        let ancho_util = ABIERTO_W - PAD * 2.0;
        if self.vista == Vista::Menu {
            return self.con_cabecera(self.menu(), w, h);
        }
        let cuerpo: PanelElement<'_> = match self.estado.clase {
            Clase::Player => {
                let extendiendo_cola = self.avance_relevo().is_some_and(|(r, _)| r.extender);
                let con_cola =
                    (self.vista == Vista::Cola || extendiendo_cola) && self.filas_cola() > 0;
                let mut cuerpo = column![
                    container(self.cabecera_player()).height(Length::Fixed(CAB_H)),
                    Space::new().height(Length::Fixed(CTRL_Y - PAD - CAB_H)),
                    self.controles(color),
                    Space::new().height(Length::Fixed(BARRA_Y - CTRL_Y - CTRL_H)),
                    self.fila_progreso(color),
                    Space::new().height(Length::Fixed(12.0)),
                    divisor(),
                    Space::new().height(Length::Fixed(SEC_Y - BARRA_Y - BARRA_H - 13.0)),
                    self.fila_secundaria(color),
                ];
                if con_cola {
                    cuerpo = cuerpo
                        .push(Space::new().height(Length::Fixed(COLA_SEP + PAD)))
                        .push(self.panel_cola(color));
                }
                let h_cuerpo = if con_cola && self.vista != Vista::Cola {
                    h + self.alto_cola()
                } else {
                    h
                };
                return self.con_cabecera(cuerpo.into(), w, h_cuerpo);
            }
            Clase::Timer => {
                let vencido = self.estado.restante_ms < 0;
                let (estado, principal) = if vencido {
                    ("Tiempo acabado", "Aplazar")
                } else if self.estado.pausado {
                    ("En pausa", "Reanudar")
                } else {
                    ("En marcha", "Pausar")
                };
                // Lo transcurrido, que es lo que crece. Vencido, llena y roja.
                let hecho = if vencido {
                    1.0
                } else if self.estado.duracion_ms > 0 {
                    1.0 - self.estado.restante_ms as f32 / self.estado.duracion_ms as f32
                } else {
                    0.0
                };
                column![
                    rotulo("Temporizador"),
                    Space::new().height(Length::Fixed(6.0)),
                    // Open Sans trae las cifras tabulares por defecto: centrada,
                    // la cuenta atrás no baila al cambiar de dígito.
                    container(
                        text(formato_tiempo(self.estado.restante_ms))
                            .size(44)
                            .font(gorda())
                            .color(if vencido { tema::rojo() } else { tema::texto() })
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(CIFRA_H))
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
                    Space::new().height(Length::Fixed(2.0)),
                    container(text(estado).size(13).color(tema::TEXTO2))
                        .width(Length::Fill)
                        .height(Length::Fixed(ESTADO_H))
                        .align_x(Horizontal::Center),
                    Space::new().height(Length::Fixed(14.0)),
                    // Sin duración publicada no hay progreso que enseñar: el
                    // hueco se queda para que los botones no se muevan.
                    if self.estado.duracion_ms > 0 || vencido {
                        barra(
                            hecho,
                            if vencido {
                                tema::rojo()
                            } else {
                                tema::amarillo()
                            },
                        )
                    } else {
                        Space::new().height(Length::Fixed(BARRA_GRUESO)).into()
                    },
                    Space::new().height(Length::Fixed(18.0)),
                    row![
                        boton(principal, Boton::Principal),
                        boton("Parar", Boton::Secundario)
                    ]
                    .spacing(BOTON_SEP),
                ]
                .into()
            }
            Clase::Transfer => {
                let f = self.estado.nivel.clamp(0.0, 1.0);
                let ancho_texto = ancho_util - 44.0 - 14.0;
                column![
                    container(
                        row![
                            ficha(DOCUMENTO),
                            column![
                                text(crate::emergente::recortar_texto(
                                    &self.estado.titulo,
                                    ancho_texto,
                                    15.0
                                ))
                                .size(15)
                                .font(negrita())
                                .wrapping(iced_core::text::Wrapping::None),
                                text(crate::emergente::recortar_texto(
                                    &self.estado.subtitulo,
                                    ancho_texto,
                                    12.0
                                ))
                                .size(12)
                                .color(tema::TEXTO2)
                                .wrapping(iced_core::text::Wrapping::None),
                            ]
                            .spacing(3),
                        ]
                        .spacing(14)
                        .align_y(Vertical::Center)
                    )
                    .height(Length::Fixed(TRANSFER_CAB_H)),
                    Space::new().height(Length::Fixed(18.0)),
                    // Verde y no acento: la barra informa, no se toca.
                    barra(f, tema::verde()),
                    Space::new().height(Length::Fixed(10.0)),
                    container(
                        row![
                            text(format!("{:.0} %", f * 100.0)).size(12).font(negrita()),
                            Space::new().width(Length::Fill),
                            text(if self.estado.restante_ms >= 0 {
                                format!("Quedan {}", formato_corto(self.estado.restante_ms))
                            } else {
                                String::new()
                            })
                            .size(12)
                            .color(tema::TEXTO2),
                        ]
                        .align_y(Vertical::Center)
                    )
                    .height(Length::Fixed(TRANSFER_INFO_H)),
                    Space::new().height(Length::Fixed(18.0)),
                    boton("Cancelar", Boton::Secundario),
                ]
                .into()
            }
            Clase::Calendar => {
                let en_curso = self.estado.restante_ms < 0;
                column![
                    rotulo(if en_curso {
                        "En curso"
                    } else {
                        "Próximo evento"
                    }),
                    Space::new().height(Length::Fixed(6.0)),
                    container(
                        text(crate::emergente::recortar_texto(
                            &self.estado.titulo,
                            ancho_util,
                            20.0
                        ))
                        .size(20)
                        .font(gorda())
                        .wrapping(iced_core::text::Wrapping::None)
                    )
                    .height(Length::Fixed(CAL_TITULO_H))
                    .align_y(Vertical::Center),
                    Space::new().height(Length::Fixed(2.0)),
                    // El horario lo compone la app («10:00–11:30 · Aula 3»):
                    // aquí no se deduce ninguna hora de fin ni lugar.
                    container(
                        text(crate::emergente::recortar_texto(
                            &self.estado.subtitulo,
                            ancho_util,
                            14.0
                        ))
                        .size(14)
                        .color(tema::TEXTO2)
                        .wrapping(iced_core::text::Wrapping::None)
                    )
                    .height(Length::Fixed(CAL_SUB_H)),
                    Space::new().height(Length::Fixed(12.0)),
                    container(
                        row![
                            icono_teñido(icono_inline(RELOJ), 16.0, tema::TEXTO2),
                            text(if en_curso {
                                "Ya ha empezado".to_string()
                            } else {
                                format!("Empieza {}", cuenta_atras(self.estado.restante_ms))
                            })
                            .size(13)
                            .color(tema::TEXTO2),
                        ]
                        .spacing(8)
                        .align_y(Vertical::Center)
                    )
                    .height(Length::Fixed(CAL_CUANDO_H)),
                    Space::new().height(Length::Fixed(18.0)),
                    // La única orden que el calendario atiende es «dismiss»
                    // (calendar/src-tauri/src/avisos.rs); un «Abrir
                    // calendario» no tendría quien lo escuchase.
                    boton("Descartar", Boton::Principal),
                ]
                .into()
            }
            Clase::Auriculares => {
                let n = self.estado.metricas.len().min(3);
                column![
                    container(
                        row![
                            image(FOTOS_BUDS[3].clone())
                                .width(AUR_CAB_H)
                                .height(AUR_CAB_H),
                            text(crate::emergente::recortar_texto(
                                &self.estado.titulo,
                                ancho_util - AUR_CAB_H - 12.0,
                                17.0
                            ))
                            .size(17)
                            .font(gorda())
                            .wrapping(iced_core::text::Wrapping::None),
                        ]
                        .spacing(12)
                        .align_y(Vertical::Center)
                    )
                    .height(Length::Fixed(AUR_CAB_H)),
                    Space::new().height(Length::Fixed(8.0)),
                    filas_metricas(
                        &self.estado.metricas,
                        // El orden lo fija bookos-budsd: izquierdo, derecho y,
                        // si informa, el estuche.
                        &FOTOS_BUDS[..3],
                        true
                    ),
                    // Sin estuche quedan dos filas: el botón no se mueve,
                    // que es donde `pulsar` lo busca.
                    Space::new().height(Length::Fixed(MON_FILA_H * (3 - n) as f32 + 16.0)),
                    match self.estado.anc {
                        Some(modo) => segmentado_ruido(modo),
                        None => boton("Abrir en Ajustes", Boton::Principal),
                    },
                ]
                .into()
            }
            Clase::Asistente(_) => Space::new().into(),
            Clase::Monitor => column![
                rotulo("Rendimiento"),
                Space::new().height(Length::Fixed(8.0)),
                filas_metricas(&self.estado.metricas, &[], false),
            ]
            .into(),
            Clase::Recorder => column![
                container(
                    row![
                        punto(self.sonando(), tema::rojo(), 10.0),
                        text(if self.estado.pausado {
                            "En pausa"
                        } else {
                            "Grabando"
                        })
                        .size(13)
                        .font(negrita()),
                    ]
                    .spacing(8)
                    .align_y(Vertical::Center)
                )
                .height(Length::Fixed(REC_CAB_H)),
                Space::new().height(Length::Fixed(6.0)),
                container(
                    text(formato_tiempo(self.estado.posicion_ms))
                        .size(34)
                        .font(gorda())
                )
                .width(Length::Fill)
                .height(Length::Fixed(REC_TIEMPO_H))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
                Space::new().height(Length::Fixed(12.0)),
                container(ondas(
                    self.estado.nivel,
                    self.sonando(),
                    tema::rojo(),
                    REC_ONDAS_H,
                    29,
                    5.0
                ))
                .width(Length::Fill)
                .height(Length::Fixed(REC_ONDAS_H))
                .align_x(Horizontal::Center),
                Space::new().height(Length::Fixed(18.0)),
                // Pausa con el peso de un control secundario; detener, en
                // rojo, porque cierra la grabación.
                container(
                    row![
                        redondo(
                            icono_inline(if self.estado.pausado { PLAY } else { PAUSA }),
                            REC_BOTON,
                            Fondo::Tenue,
                        ),
                        redondo(icono_inline(STOP), REC_BOTON, Fondo::Solido(tema::rojo())),
                    ]
                    .spacing(28)
                )
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            ]
            .into(),
        };
        self.con_cabecera(cuerpo, w, h)
    }

    /// La tarjeta con su franja de cabecera encima del contenido.
    // añadido en la 0.2.2
    // modificado en la 0.2.3 -- con columna, sin fondo ni sombra propios
    fn con_cabecera<'a>(&'a self, cuerpo: PanelElement<'a>, w: f32, h: f32) -> PanelElement<'a> {
        let relleno = [PAD + CAB_EXTRA, PAD, PAD, PAD];
        // Con la columna al lado, el fondo y la sombra los pone
        // `con_su_columna` para las dos. Si la tarjeta trae los suyos, su
        // sombra cae sobre la columna y su esquina redonda se ve encima: dos
        // piezas superpuestas en vez de una.
        let tarjeta = if self.con_columna() {
            container(cuerpo)
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .padding(iced_core::Padding {
                    top: relleno[0],
                    right: relleno[1],
                    bottom: relleno[2],
                    left: relleno[3],
                })
                .into()
        } else {
            tarjeta_v(cuerpo, w, h, RADIO, relleno)
        };
        stack![tarjeta, self.cabecera_app(w)].into()
    }

    /// «App ›» y ⋯. Sin placa: en la isla no hay puntero que la encienda, y
    /// una placa fija competiría con el contenido.
    // añadido en la 0.2.2
    fn cabecera_app(&self, w: f32) -> PanelElement<'_> {
        let mas_activo = self.vista == Vista::Menu;
        let mas = container(icono_teñido(
            icono_inline(MAS),
            18.0,
            if mas_activo {
                tema::texto()
            } else {
                tema::TEXTO2
            },
        ))
        .width(Length::Fixed(MAS_W))
        .height(Length::Fixed(MAS_H))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: mas_activo.then(|| Color::from_rgb8(0x1c, 0x1c, 0x1e).into()),
            border: Border {
                radius: (MAS_H / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let yo = Burbuja {
            app_id: String::new(),
            clase: self.estado.clase,
            color: color_burbuja(&self.estado),
            activa: true,
            actual: true,
        };
        container(
            row![
                Space::new().width(Length::Fixed(4.0)),
                icono_columna(&yo, CAB_ICONO),
                Space::new().width(Length::Fixed(7.0)),
                text(self.app_nombre.clone())
                    .size(12)
                    .font(negrita())
                    .color(tema::TEXTO2)
                    .wrapping(iced_core::text::Wrapping::None),
                Space::new().width(Length::Fixed(7.0)),
                icono_teñido(icono_propio("chevron-derecha"), 12.0, apagado(0.6)),
                Space::new().width(Length::Fill),
                mas,
            ]
            .align_y(Vertical::Center)
            .height(Length::Fixed(CAB_APP_H)),
        )
        .width(Length::Fixed(w))
        .padding(iced_core::Padding {
            top: CAB_APP_Y,
            right: MAS_DER,
            bottom: 0.0,
            left: CAB_APP_X,
        })
        .into()
    }

    /// El menú de la actividad: abrir la app, ponerla primera y ocultarla.
    // añadido en la 0.2.2
    fn menu(&self) -> PanelElement<'_> {
        let primera = self.es_la_primera();
        let fila = |svg: &str, etiqueta: String, apagada: bool| -> PanelElement<'static> {
            let tinta = if apagada { tema::TEXTO2 } else { tema::texto() };
            container(
                row![
                    icono_teñido(icono_inline(svg), 18.0, tema::TEXTO2),
                    text(etiqueta).size(14.5).font(medio()).color(tinta),
                ]
                .spacing(10)
                .align_y(Vertical::Center),
            )
            .height(Length::Fixed(MENU_FILA_H))
            .align_y(Vertical::Center)
            .into()
        };
        column![
            fila(ABRIR, format!("Abrir {}", self.app_nombre), false),
            if primera {
                fila(FIJAR, "Ya se muestra primero".into(), true)
            } else {
                fila(FIJAR, "Mostrar primero".into(), false)
            },
            fila(OCULTAR, "Ocultar de la isla".into(), false),
            // No hay permisos por app que ajustar: lleva a la página que
            // enciende las actividades y decide el temporizador.
            fila(CAMPANA, "Ajustes de actividades…".into(), false),
        ]
        .into()
    }

    fn onda_asistente(&self, alto: f32) -> PanelElement<'static> {
        let nivel = self.nivel_suave();
        let t = if self.animaciones {
            self.desde.elapsed().as_secs_f32()
        } else {
            0.0
        };
        let pensando = self.estado.clase == Clase::Asistente(Fase::Pensando);
        let mut barras = row![].spacing(3).align_y(Vertical::Center);
        for (i, perfil) in [0.45, 0.75, 1.0, 0.75, 0.45].into_iter().enumerate() {
            let h = if pensando {
                // Pensando no oye nada: un bulto que recorre las barras de ida
                // y vuelta dice «trabajando» sin fingir voz. Sin animaciones se
                // queda centrado, que sigue distinguiéndose del silencio.
                let centro = 2.0 + 2.0 * (t * 2.4).sin();
                let d = i as f32 - centro;
                3.0 + (alto - 3.0) * 0.7 * (-d * d / 1.2).exp()
            } else {
                let movimiento = if self.animaciones {
                    0.65 + 0.35 * (t * 5.0 + i as f32 * 0.8).sin()
                } else {
                    1.0
                };
                // El vúmetro musical fuerza un nivel mínimo de 0,8: aquí el
                // silencio tiene que verse para no simular que se está oyendo voz.
                // En silencio late apenas: da señal de vida sin fingir voz.
                let reposo = if self.animaciones {
                    1.5 * (0.5 + 0.5 * (t * 2.2 - i as f32 * 0.6).sin())
                } else {
                    0.0
                };
                3.0 + reposo + (alto - 3.0) * perfil * nivel * movimiento
            };
            // Como en la Siri de iOS 27, la onda es de lo poco con color: el
            // resto de la píldora es oscuro y el color se va de barra en barra.
            let fase = 0.5 + 0.5 * (t * 1.1 - i as f32 * 0.7).sin();
            let color = tema::mezclar(
                tema::mezclar(self.tonos[0], self.tonos[1], i as f32 / 4.0),
                self.tonos[2],
                fase * 0.6,
            );
            barras = barras.push(
                container(Space::new())
                    .width(3)
                    .height(h)
                    .style(move |_| pill(color)),
            );
        }
        container(barras)
            .width(ASIS_ONDA_W)
            .height(alto)
            .align_y(Vertical::Center)
            .into()
    }

    fn pildora_asistente(&self, fase: Fase) -> PanelElement<'_> {
        if fase == Fase::Minimizado {
            let disco = container(self.onda_asistente(22.0))
                .width(ASIS_MINI - 4.0)
                .height(ASIS_MINI - 4.0)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(|_| container::Style {
                    background: Some(tema::card().into()),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            return container(stack![
                relleno_vivo(self.tonos, self.giro(), ASIS_MINI, ASIS_MINI, tema::R_PILL),
                container(disco)
                    .width(ASIS_MINI)
                    .height(ASIS_MINI)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
            ])
            .style(|_| container::Style {
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                shadow: tema::sombra_popover(),
                ..Default::default()
            })
            .into();
        }
        let estado = match fase {
            Fase::Escuchando => "Te escucho…",
            Fase::Pensando => "Pensando…",
            Fase::Hecho => "Listo",
            Fase::Respuesta => "Respuesta de Zenith",
            Fase::Error => "No he podido continuar",
            Fase::Minimizado => unreachable!("la burbuja se dibuja antes de la píldora"),
        };
        fn vacio_o<'a>(a: &'a str, b: &'a str) -> &'a str {
            if a.is_empty() { b } else { a }
        }
        // Una sola línea: lo que más importa ahora. Pensando importa lo que está
        // haciendo («Transcribiendo…»); lo dicho ya no hace falta repetirlo.
        let linea = match fase {
            Fase::Pensando => vacio_o(&self.estado.subtitulo, estado),
            _ => vacio_o(&self.estado.titulo, vacio_o(&self.estado.subtitulo, estado)),
        };
        let con_onda = matches!(fase, Fase::Escuchando | Fase::Pensando);
        let con_cierre = fase != Fase::Hecho;
        let ancho = ASIS_COMPACTO_W
            - ASIS_LADO * 2.0
            - ASIS_ORBE
            - ASIS_SEP
            - if con_onda {
                ASIS_ONDA_W + ASIS_SEP
            } else {
                0.0
            }
            - if con_cierre {
                ASIS_CERRAR_PILDORA + ASIS_SEP
            } else {
                0.0
            };
        let marca = match fase {
            Fase::Hecho => redondo(icono_inline(VISTO), ASIS_ORBE, Fondo::Solido(tema::verde())),
            Fase::Error => redondo(icono_inline(AVISO), ASIS_ORBE, Fondo::Solido(tema::rojo())),
            _ => {
                // El orbe late con la voz; pensando, por su cuenta y despacio.
                let t = self.desde.elapsed().as_secs_f32();
                let pulso = match fase {
                    Fase::Pensando if self.animaciones => 0.5 + 0.5 * (t * 2.0).sin(),
                    _ => self.nivel_suave(),
                };
                let lado = ASIS_ORBE * (0.72 + 0.28 * pulso);
                container(relleno_vivo(
                    self.tonos,
                    self.giro(),
                    lado,
                    lado,
                    lado / 2.0,
                ))
                .width(ASIS_ORBE)
                .height(ASIS_ORBE)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .into()
            }
        };
        let texto_linea = if fase == Fase::Escuchando {
            recortar_final(linea, ancho, 13.0, medio())
        } else {
            recortar_asistente(linea, ancho, 13.0, medio())
        };
        let mut contenido = row![
            marca,
            container(
                text(texto_linea)
                    .size(13)
                    .font(medio())
                    .wrapping(iced_core::text::Wrapping::None),
            )
            .width(Length::Fill),
        ]
        .spacing(ASIS_SEP)
        .align_y(Vertical::Center);
        if con_onda {
            contenido = contenido.push(self.onda_asistente(16.0));
        }
        if con_cierre {
            contenido = contenido.push(redondo(
                icono_inline(CRUZ),
                ASIS_CERRAR_PILDORA,
                Fondo::Tenue,
            ));
        }
        let fondo = isla();
        let cuerpo = container(contenido)
            .width(ASIS_COMPACTO_W - ASIS_CANTO * 2.0)
            .height(ASIS_COMPACTO_H - ASIS_CANTO * 2.0)
            .padding([0.0, ASIS_LADO - ASIS_CANTO])
            .align_y(Vertical::Center)
            .style(move |_| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        // Stack de opacos y no un borde: iced no admite degradado en el borde, y
        // el relleno translúcido en tiny-skia cuesta 17 veces más que el opaco.
        container(stack![
            relleno_vivo(
                self.tonos,
                self.giro(),
                ASIS_COMPACTO_W,
                ASIS_COMPACTO_H,
                tema::R_PILL,
            ),
            container(cuerpo)
                .width(ASIS_COMPACTO_W)
                .height(ASIS_COMPACTO_H)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        ])
        .style(|_| container::Style {
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            shadow: tema::sombra_popover(),
            ..Default::default()
        })
        .into()
    }

    /// El ángulo del color vivo. Se calcula con la fase actual y no se acumula:
    /// al cambiar de fase da un salto, que queda dentro del pulso de ese cambio.
    // añadido en la 0.2.0
    fn giro(&self) -> f32 {
        if !self.animaciones {
            return std::f32::consts::FRAC_PI_4;
        }
        let vel = if self.estado.clase == Clase::Asistente(Fase::Pensando) {
            ASIS_GIRO_PIENSA
        } else {
            ASIS_GIRO_ESCUCHA
        };
        self.desde.elapsed().as_secs_f32() * vel
    }

    fn abierta_asistente(&self, fase: Fase) -> PanelElement<'_> {
        let ancho_util = ABIERTO_W - PAD * 2.0;
        let error = fase == Fase::Error;
        let marca = if error {
            redondo(icono_inline(AVISO), ASIS_ORBE, Fondo::Solido(tema::rojo()))
        } else {
            orbe(self.tonos, ASIS_ORBE)
        };
        let cuerpo = column![
            row![
                marca,
                column![
                    text("Zenith").size(13).font(negrita()),
                    text(if error {
                        "Necesita atención"
                    } else {
                        "Respuesta"
                    })
                    .size(11)
                    .color(tema::mezclar(tema::texto(), tema::TEXTO2, 0.75)),
                ]
                .spacing(2)
                .width(Length::Fill),
                redondo(icono_inline(CRUZ), ASIS_CERRAR, Fondo::Tenue),
            ]
            .spacing(ASIS_SEP)
            .align_y(Vertical::Center)
            .height(ASIS_MARCA_H),
            Space::new().height(16),
            container(
                text(recortar_asistente(
                    if self.estado.titulo.is_empty() {
                        if error {
                            "No he podido continuar"
                        } else {
                            "Respuesta de Zenith"
                        }
                    } else {
                        &self.estado.titulo
                    },
                    ancho_util,
                    17.0,
                    gorda(),
                ))
                .size(17)
                .font(gorda())
                .wrapping(iced_core::text::Wrapping::None)
            )
            .height(ASIS_TITULO_H),
            Space::new().height(if self.cuerpo_asistente.is_empty() {
                0.0
            } else {
                8.0
            }),
            container(
                text(&self.cuerpo_asistente)
                    .size(tema::T_CUERPO)
                    .line_height(iced_core::text::LineHeight::Absolute(ASIS_LINEA_H.into()))
                    .wrapping(iced_core::text::Wrapping::None)
                    .color(tema::mezclar(tema::texto(), tema::TEXTO2, 0.75))
            )
            .height(self.alto_cuerpo_asistente()),
            Space::new().height(20),
            boton_con(
                if error { "Reintentar" } else { "Entendido" },
                self.tonos[0],
                self.sobre_tono,
            ),
        ];
        superficie_asistente(
            cuerpo.into(),
            ABIERTO_W,
            self.boton_asistente_y() + BOTON_H + PAD,
            tema::R_DIALOGO,
            [PAD; 4],
            self.tonos[0],
            tema::card(),
        )
    }

    // 0.1.0
    fn cabecera_player(&self) -> PanelElement<'_> {
        let ancho = ABIERTO_W - PAD * 2.0 - ARTE - 16.0;
        row![
            caratula(self.arte.as_ref(), ARTE),
            column![
                text(crate::emergente::recortar_texto(
                    &self.estado.titulo,
                    ancho,
                    17.0
                ))
                .size(17)
                .font(negrita())
                .wrapping(iced_core::text::Wrapping::None),
                text(crate::emergente::recortar_texto(
                    &self.estado.subtitulo,
                    ancho,
                    14.0
                ))
                .size(14)
                .color(tema::TEXTO2)
                .wrapping(iced_core::text::Wrapping::None),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(16)
        .align_y(Vertical::Center)
        .into()
    }

    // 0.1.0
    fn controles(&self, color: Color) -> PanelElement<'_> {
        let central = if self.estado.pausado { PLAY } else { PAUSA };
        let plano = |activo: bool| if activo { color } else { tema::TEXTO2 };
        container(
            row![
                icono_teñido(icono_inline(ALEATORIO), 20.0, plano(self.estado.aleatorio)),
                // Como la referencia: solo el play lleva placa, del color de la
                // tinta del tema (blanca con el glifo oscuro en oscuro, como
                // siempre; oscura en claro). Los saltos, sueltos.
                redondo(icono_inline(ANTERIOR), CTRL_LADO, Fondo::Sin),
                redondo(
                    icono_inline(central),
                    CTRL_PLAY,
                    Fondo::Solido(tema::texto())
                ),
                redondo(icono_inline(SIGUIENTE), CTRL_LADO, Fondo::Sin),
                self.favorita_actual(plano),
            ]
            .spacing(CTRL_SEP)
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(CTRL_H))
        .align_x(Horizontal::Center)
        .into()
    }

    /// El corazón de la que suena. Sin cola publicada no se sabe cuál es, y el
    /// hueco queda vacío en vez de un botón que no hace nada.
    // añadido en la 0.2.2
    fn favorita_actual(&self, plano: impl Fn(bool) -> Color) -> PanelElement<'_> {
        match self.estado.cola.iter().find(|i| i.actual) {
            Some(i) => icono_teñido(
                icono_inline(if i.favorita { CORAZON_LLENO } else { CORAZON }),
                20.0,
                plano(i.favorita),
            ),
            None => Space::new()
                .width(Length::Fixed(20.0))
                .height(Length::Fixed(20.0))
                .into(),
        }
    }

    // 0.1.0
    fn fila_progreso(&self, color: Color) -> PanelElement<'_> {
        let (pos, total) = (self.estado.posicion_ms, self.estado.duracion_ms);
        let f = if total > 0 {
            (pos.max(0) as f32 / total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let tiempo = |ms: i64| text(formato_corto(ms)).size(12).color(tema::TEXTO2);
        container(
            row![
                container(tiempo(pos)).width(Length::Fixed(TIEMPO_W)),
                barra(f, color),
                // Lo que queda, como la maqueta: es lo que se mira de reojo.
                container(
                    text(format!("-{}", formato_corto((total - pos).max(0))))
                        .size(12)
                        .color(tema::TEXTO2)
                )
                .width(Length::Fixed(TIEMPO_W))
                .align_x(Horizontal::Right),
            ]
            .spacing(10)
            .align_y(Vertical::Center),
        )
        .height(Length::Fixed(BARRA_H))
        .align_y(Vertical::Center)
        .into()
    }

    /// Cola y volumen, cada uno en su mitad. Pesan menos que el progreso:
    /// texto secundario, iconos grises, y el acento solo en lo que está
    /// encendido o se arrastra.
    // 0.1.0
    fn fila_secundaria(&self, color: Color) -> PanelElement<'_> {
        let tinta_cola = if self.filas_cola() == 0 {
            apagado(0.3)
        } else if self.vista == Vista::Cola {
            color
        } else {
            tema::TEXTO2
        };
        container(
            row![
                container(
                    row![
                        icono_teñido(icono_propio("cola-lista"), 18.0, tinta_cola),
                        text("A continuación")
                            .size(13)
                            .font(medio())
                            .color(tinta_cola),
                    ]
                    .spacing(10)
                    .align_y(Vertical::Center)
                )
                .width(Length::Fixed(ABIERTO_W / 2.0 - PAD)),
                Space::new().width(Length::Fixed(16.0)),
                icono_teñido(icono_inline(VOLUMEN), ICONO_FILA, tema::TEXTO2),
                Space::new().width(Length::Fixed(10.0)),
                deslizador(self.estado.volumen, color),
            ]
            .align_y(Vertical::Center),
        )
        .height(Length::Fixed(SEC_H))
        .align_y(Vertical::Center)
        .into()
    }

    // 0.1.0
    fn panel_cola(&self, color: Color) -> PanelElement<'_> {
        let mut filas = column![
            container(
                text("A continuación")
                    .size(12)
                    .font(medio())
                    .color(tema::TEXTO2)
            )
            .height(Length::Fixed(COLA_CAB_H))
            .align_y(Vertical::Center),
        ];
        for item in self.cola_visible() {
            let actual = item.actual;
            filas = filas.push(
                container(
                    row![
                        // Sin carátula propia, el color del disco: un
                        // cuadrado gris en cada fila se ve como un hueco sin
                        // cargar.
                        match self.miniaturas.get(&item.id) {
                            Some(m) => caratula(Some(m), 42.0),
                            None if actual => caratula(self.arte.as_ref(), 42.0),
                            // Sobre el gris de la cola, el hueco va del color
                            // de la tarjeta: con el mismo gris desaparecía.
                            None => caratula_vacia(42.0, tema::card()),
                        },
                        column![
                            text(item.titulo.clone())
                                .size(14)
                                .font(negrita())
                                .color(if actual { color } else { tema::texto() }),
                            text(item.artista.clone()).size(12).color(tema::TEXTO2),
                        ]
                        .spacing(3)
                        .width(Length::Fill),
                        text(if item.duracion_ms > 0 {
                            formato_corto(item.duracion_ms)
                        } else {
                            String::new()
                        })
                        .size(12)
                        .color(tema::TEXTO2),
                        icono_teñido(
                            icono_inline(if item.favorita {
                                CORAZON_LLENO
                            } else {
                                CORAZON
                            }),
                            18.0,
                            if item.favorita { color } else { apagado(0.45) },
                        ),
                    ]
                    .spacing(13)
                    .align_y(Vertical::Center),
                )
                .height(Length::Fixed(COLA_FILA_H)),
            );
        }
        container(filas)
            .width(Length::Fill)
            .padding(iced_core::Padding {
                top: COLA_PAD,
                right: COLA_PAD,
                bottom: COLA_PAD,
                left: COLA_PAD,
            })
            // Opaca y compuesta sobre la tarjeta: un velo translúcido de este
            // tamaño cuesta 17 veces más en tiny-skia (ver `superficie_en_tarjeta`).
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    // Más cerrado que la tarjeta que lo contiene, para que se
                    // lea como un contenedor dentro de otro.
                    radius: tema::R_POPOVER.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

// ── Piezas ───────────────────────────────────────────────────────────────

fn recortar_asistente(texto: &str, ancho: f32, tamaño: f32, fuente: Font) -> String {
    let texto = texto.split_whitespace().collect::<Vec<_>>().join(" ");
    let medir = |s: &str| crate::widget::ancho_con_fuente(s, tamaño, fuente);
    if medir(&texto) <= ancho {
        return texto;
    }
    // El ancho medio por carácter deja que una transcripción con letras
    // anchas invada el vúmetro y el cierre. Se mide con el peso que se pinta.
    let limite = ancho - medir("…");
    let cortes: Vec<usize> = texto.char_indices().map(|(i, _)| i).collect();
    let i = cortes
        .partition_point(|&i| medir(&texto[..i]) <= limite)
        .saturating_sub(1);
    format!("{}…", texto[..cortes[i]].trim_end())
}

/// Como [`recortar_asistente`], pero guarda el final: mientras se dicta lo que
/// interesa es la última palabra, y cortando por detrás el texto se quedaba
/// clavado con la frase aún en marcha.
// añadido en la 0.2.0
fn recortar_final(texto: &str, ancho: f32, tamaño: f32, fuente: Font) -> String {
    let texto = texto.split_whitespace().collect::<Vec<_>>().join(" ");
    let medir = |s: &str| crate::widget::ancho_con_fuente(s, tamaño, fuente);
    if medir(&texto) <= ancho {
        return texto;
    }
    let limite = ancho - medir("…");
    let cortes: Vec<usize> = texto.char_indices().map(|(i, _)| i).collect();
    let i = cortes.partition_point(|&i| medir(&texto[i..]) > limite);
    let i = cortes.get(i).copied().unwrap_or(texto.len());
    format!("…{}", texto[i..].trim_start())
}

// 0.1.0
fn tarjeta<'a>(
    contenido: PanelElement<'a>,
    w: f32,
    h: f32,
    radio: f32,
    padding: [u16; 2],
) -> PanelElement<'a> {
    let (v, hh) = (padding[0] as f32, padding[1] as f32);
    tarjeta_v(contenido, w, h, radio, [v, hh, v, hh])
}

/// La tarjeta, con el padding por lados.
///
/// El fondo es liso, como el del resto del escritorio (panel, dock, lanzador,
/// avisos): un degradado hacía de la isla la única superficie que brillaba. La
/// sombra va **también en oscuro**: la isla cuelga sobre el escritorio, y sin
/// ella se confunde con una ventana pegada al borde.
// 0.1.0
fn tarjeta_v<'a>(
    contenido: PanelElement<'a>,
    w: f32,
    h: f32,
    radio: f32,
    padding: [f32; 4],
) -> PanelElement<'a> {
    // Se lee al construir: el estilo se evalúa al pintar, con la bandera ya
    // devuelta a su sitio.
    let con_fondo = !sin_fondo();
    container(contenido)
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        // `Padding` no se construye desde un array de cuatro; se arma por
        // lados, que además dice cuál es cuál.
        .padding(iced_core::Padding {
            top: padding[0],
            right: padding[1],
            bottom: padding[2],
            left: padding[3],
        })
        .style(move |_| {
            if !con_fondo {
                return container::Style::default();
            }
            container::Style {
                background: Some(isla().into()),
                border: Border {
                    radius: radio.into(),
                    ..Default::default()
                },
                shadow: tema::sombra_popover(),
                ..Default::default()
            }
        })
        .into()
}

/// El filo conserva el tono del fondo; la sombra deja separar la isla de
/// las ventanas sin convertir toda la tarjeta en un indicador luminoso.
// añadido en la 0.2.0
fn superficie_asistente<'a>(
    contenido: PanelElement<'a>,
    w: f32,
    h: f32,
    radio: f32,
    padding: [f32; 4],
    tono: Color,
    fondo: Color,
) -> PanelElement<'a> {
    container(contenido)
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .padding(iced_core::Padding {
            top: padding[0],
            right: padding[1],
            bottom: padding[2],
            left: padding[3],
        })
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: radio.into(),
                width: 1.0,
                color: tema::alfa(tono, if tema::alto_contraste() { 1.0 } else { 0.22 }),
            },
            shadow: Shadow {
                color: tema::alfa(tono, 0.12),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 22.0,
            },
            ..Default::default()
        })
        .into()
}

/// Un degradado que gira, opaco: el canto de la píldora, el aro de la burbuja
/// y el orbe.
// añadido en la 0.2.0
fn relleno_vivo(tonos: [Color; 3], giro: f32, w: f32, h: f32, radio: f32) -> PanelElement<'static> {
    let degradado = iced_core::gradient::Linear::new(iced_core::Radians(giro))
        .add_stop(0.0, tonos[0])
        .add_stop(0.5, tonos[1])
        .add_stop(1.0, tonos[2]);
    container(Space::new())
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .style(move |_| container::Style {
            background: Some(degradado.into()),
            border: Border {
                radius: radio.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// La marca usa los mismos tonos en la isla y en el aro de la burbuja.
// añadido en la 0.2.0
fn orbe(tonos: [Color; 3], lado: f32) -> PanelElement<'static> {
    let degradado =
        iced_core::gradient::Linear::new(iced_core::Radians(std::f32::consts::FRAC_PI_4))
            .add_stop(0.0, tonos[0])
            .add_stop(0.55, tonos[1])
            .add_stop(1.0, tonos[2]);
    let tinta = tema::tinta_sobre(tema::mezclar(tonos[0], tonos[1], 0.5));
    container(icono_teñido(icono_propio("micro"), lado * 0.5, tinta))
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(degradado.into()),
            border: Border {
                radius: (lado / 2.0).into(),
                width: 1.0,
                color: tema::alfa(tema::texto(), 0.12),
            },
            ..Default::default()
        })
        .into()
}

/// La carátula, o un cuadrado de superficie con una nota cuando no hay
/// ninguna. Sin sombra de color: era un halo que cambiaba con cada disco.
// 0.1.0
fn caratula(arte: Option<&image::Handle>, lado: f32) -> PanelElement<'static> {
    match arte {
        Some(h) => image(h.clone()).width(lado).height(lado).into(),
        None => caratula_vacia(lado, tema::superficie_en_tarjeta()),
    }
}

// 0.1.0
fn caratula_vacia(lado: f32, fondo: Color) -> PanelElement<'static> {
    container(icono_teñido(
        icono_propio("musica"),
        lado * 0.46,
        tema::TEXTO2,
    ))
    .width(Length::Fixed(lado))
    .height(Length::Fixed(lado))
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .style(move |_| container::Style {
        background: Some(fondo.into()),
        border: Border {
            radius: (lado * 0.22).into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

/// El icono de un archivo en su ficha, como los iconos de fila de Ajustes.
// 0.1.0
fn ficha(svg: &str) -> PanelElement<'static> {
    container(icono_teñido(icono_inline(svg), 22.0, tema::texto()))
        .width(Length::Fixed(44.0))
        .height(Length::Fixed(44.0))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_BOTON.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Icono, nombre recortado y el dato a la derecha: la píldora de las
/// actividades que identifican algo con un nombre.
// 0.1.0
fn fila_compacta(
    icono: PanelElement<'static>,
    titulo: &str,
    ancho_texto: f32,
    dato: String,
    color_dato: Color,
) -> PanelElement<'static> {
    row![
        icono,
        text(crate::emergente::recortar_texto(
            titulo,
            ancho_texto - 16.0 - 16.0 - 24.0,
            12.0
        ))
        .size(12)
        .font(negrita())
        .wrapping(iced_core::text::Wrapping::None)
        .width(Length::Fill),
        text(dato).size(12).color(color_dato),
    ]
    .spacing(8)
    .align_y(Vertical::Center)
    .into()
}

#[derive(Clone, Copy, PartialEq)]
enum Fondo {
    /// Relleno de color: el control principal (acento) o detener (rojo).
    Solido(Color),
    /// Superficie neutra con el icono en el color del texto: los de alrededor.
    Tenue,
    /// Sin placa: solo el glifo. Anterior y siguiente de la música, como la
    /// referencia; la placa queda para la acción principal.
    // añadido en la 0.2.0
    Sin,
}

// 0.1.0
fn redondo(ic: Option<icono::Icono>, lado: f32, fondo: Fondo) -> PanelElement<'static> {
    let (relleno, tinta) = match fondo {
        Fondo::Solido(c) => (c, tema::tinta_sobre(c)),
        Fondo::Tenue => (tema::superficie_en_tarjeta(), tema::texto()),
        Fondo::Sin => (Color::TRANSPARENT, tema::texto()),
    };
    // Sin placa el glifo es lo único que se ve: algo más grande.
    let px = if matches!(fondo, Fondo::Sin) {
        lado * 0.5
    } else {
        lado * 0.4
    };
    container(icono_teñido(ic, px, tinta))
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(relleno.into()),
            border: Border {
                radius: (lado / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

#[derive(Clone, Copy)]
enum Boton {
    /// Acento con su tinta: la acción de la tarjeta.
    Principal,
    /// Superficie neutra: la alternativa.
    Secundario,
}

/// Botón de diálogo del sistema de diseño: 48 de alto, radio de control,
/// 15/Semibold, repartido a partes iguales con su pareja.
// 0.1.0
fn boton(rotulo: &'static str, tipo: Boton) -> PanelElement<'static> {
    let (relleno, tinta) = match tipo {
        Boton::Principal => (tema::acento(), tema::sobre_acento()),
        Boton::Secundario => (tema::superficie_en_tarjeta(), tema::texto()),
    };
    boton_con(rotulo, relleno, tinta)
}

/// El mismo botón con sus colores: el asistente usa el acento de la paleta del
/// fondo y no el del sistema.
// añadido en la 0.2.0
fn boton_con(rotulo: &'static str, relleno: Color, tinta: Color) -> PanelElement<'static> {
    container(text(rotulo).size(15).font(negrita()).color(tinta))
        .width(Length::Fill)
        .height(Length::Fixed(BOTON_H))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(relleno.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

// 0.1.0
fn rotulo(t: &'static str) -> PanelElement<'static> {
    container(text(t).size(12).font(medio()).color(tema::TEXTO2))
        .height(Length::Fixed(ROTULO_H))
        .align_y(Vertical::Center)
        .into()
}

// 0.1.0
fn divisor() -> PanelElement<'static> {
    container(Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_| container::Style {
            background: Some(tema::divisor().into()),
            ..Default::default()
        })
        .into()
}

/// Un auricular sin lectura. bookos-budsd manda 0 cuando el aparato no
/// informa (lo dice al omitir el estuche a 0) y nunca marca crítico un 0:
/// enseñarlo como «0 %» se leería como descargado.
// 0.1.0
fn desconocida(m: &Metrica) -> bool {
    m.valor <= 0.0 && !m.critica
}

/// Una fila «nombre … cifra» por métrica, separadas por divisores. Solo
/// cifras: una barra por fila no aportaba nada a una lectura de reojo. Rojo si
/// quien publica la marca crítica.
/// `fotos` pone una delante de cada etiqueta, por posición; sin foto, la fila
/// empieza por el texto.
// 0.1.0
fn filas_metricas(
    metricas: &[Metrica],
    fotos: &[image::Handle],
    cero_desconocido: bool,
) -> PanelElement<'static> {
    let mut filas = column![];
    for (i, m) in metricas.iter().enumerate() {
        if i > 0 {
            filas = filas.push(divisor());
        }
        let (valor, c) = if cero_desconocido && desconocida(m) {
            ("—".to_string(), tema::TEXTO2)
        } else if m.critica {
            (m.texto.clone(), tema::rojo())
        } else {
            (m.texto.clone(), tema::texto())
        };
        filas = filas.push(
            container(
                row![
                    match fotos.get(i) {
                        Some(foto) => row![
                            image(foto.clone()).width(FOTO_FILA).height(FOTO_FILA),
                            Space::new().width(Length::Fixed(12.0)),
                        ]
                        .into(),
                        None => PanelElement::from(Space::new()),
                    },
                    text(m.etiqueta.clone()).size(14),
                    Space::new().width(Length::Fill),
                    text(valor).size(14).font(negrita()).color(c),
                ]
                .align_y(Vertical::Center),
            )
            // El divisor se come un píxel de la fila: el alto total sigue
            // siendo `MON_FILA_H` por métrica, que es lo que reserva `size`.
            .height(Length::Fixed(MON_FILA_H - if i > 0 { 1.0 } else { 0.0 }))
            .align_y(Vertical::Center),
        );
    }
    filas.into()
}

/// El volumen como el deslizador del sistema de diseño: pista fina, relleno
/// de acento y pulgar blanco. El pulgar recorre el ancho menos su lado, y
/// `pulsar_player` hace la misma cuenta.
// 0.1.0
fn deslizador(volumen: u8, color: Color) -> PanelElement<'static> {
    let ancho = VOL_X1 - VOL_X0;
    let x = (ancho - VOL_PULGAR) * f32::from(volumen.min(100)) / 100.0;
    let pista = container(stack![
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(4.0))
            .style(|_| pill(tema::control_apagado())),
        container(Space::new())
            .width(Length::Fixed(x + VOL_PULGAR / 2.0))
            .height(Length::Fixed(4.0))
            .style(move |_| pill(color)),
    ])
    .width(Length::Fill)
    .height(Length::Fixed(VOL_PULGAR))
    .align_y(Vertical::Center);
    let pulgar = container(Space::new())
        .width(Length::Fixed(VOL_PULGAR))
        .height(Length::Fixed(VOL_PULGAR))
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: tema::alfa(Color::BLACK, 0.2),
                offset: Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            },
            ..Default::default()
        });
    container(stack![
        pista,
        row![Space::new().width(Length::Fixed(x)), pulgar],
    ])
    .width(Length::Fixed(ancho))
    .height(Length::Fixed(VOL_PULGAR))
    .into()
}

/// Un icono de la columna: círculo del color de su actividad con el glifo en
/// blanco. Las que no son la actual se apagan hacia el negro de la isla (32 %
/// de su color): se adivinan sin competir con la que se mira.
// añadido en la 0.2.0
/// Órdenes de la isla misma, no de la app: las intercepta el shell.
// añadido en la 0.2.2
pub const ISLA_ABRIR: &str = "isla:abrir";
pub const ISLA_PRIMERO: &str = "isla:primero";
pub const ISLA_OCULTAR: &str = "isla:ocultar";
pub const ISLA_AJUSTES: &str = "isla:ajustes";

/// El nombre de la app para la cabecera: el de su `.desktop`, que es como la
/// llama el resto del escritorio. Las apps de BookOS publican con el
/// identificador de Tauri (`com.bookos.player`) y su fichero se llama como el
/// binario (`bookos-player.desktop`), así que se prueban los dos.
// añadido en la 0.2.2
fn nombre_app(estado: &Estado) -> String {
    if estado.clase == Clase::Auriculares {
        return "Ajustes".into();
    }
    let cola = estado.app_id.rsplit('.').next().unwrap_or(&estado.app_id);
    crate::apps::por_app_id(&estado.app_id)
        .or_else(|| crate::apps::por_app_id(&format!("bookos-{cola}")))
        .map(|a| a.nombre)
        .unwrap_or_else(|| {
            match estado.clase {
                Clase::Player => "Música",
                Clase::Timer => "Reloj",
                Clase::Recorder => "Grabadora",
                Clase::Transfer => "Archivos",
                Clase::Calendar => "Calendario",
                Clase::Monitor => "Monitor",
                Clase::Auriculares => "Auriculares",
                Clase::Asistente(_) => "Zenith",
            }
            .to_string()
        })
}

const CAMPANA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M5.25 9a6.75 6.75 0 0 1 13.5 0v.75c0 2.123.8 4.057 2.118 5.52a.75.75 0 0 1-.297 1.206c-1.544.57-3.16.99-4.831 1.243a3.75 3.75 0 1 1-7.48 0 24.585 24.585 0 0 1-4.831-1.244.75.75 0 0 1-.298-1.205A8.217 8.217 0 0 0 5.25 9.75V9Zm4.502 8.9a2.25 2.25 0 1 0 4.496 0 25.057 25.057 0 0 1-4.496 0Z"/></svg>"##;
const MAS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M4.5 12a1.5 1.5 0 1 1 3 0 1.5 1.5 0 0 1-3 0Zm6 0a1.5 1.5 0 1 1 3 0 1.5 1.5 0 0 1-3 0Zm6 0a1.5 1.5 0 1 1 3 0 1.5 1.5 0 0 1-3 0Z"/></svg>"##;
const ABRIR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M15.75 2.25H21a.75.75 0 0 1 .75.75v5.25a.75.75 0 0 1-1.5 0V4.81L8.03 17.03a.75.75 0 0 1-1.06-1.06L19.19 3.75h-3.440a.75.75 0 0 1 0-1.5Zm-10.5 4.5a1.5 1.5 0 0 0-1.5 1.5v10.5a1.5 1.5 0 0 0 1.5 1.5h10.5a1.5 1.5 0 0 0 1.5-1.5V10.5a.75.75 0 0 1 1.5 0v8.25a3 3 0 0 1-3 3H5.25a3 3 0 0 1-3-3V8.25a3 3 0 0 1 3-3h8.25a.75.75 0 0 1 0 1.5H5.25Z"/></svg>"##;
const FIJAR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M6.32 2.577a49.255 49.255 0 0 1 11.36 0c1.497.174 2.57 1.46 2.57 2.93V21a.75.75 0 0 1-1.085.67L12 18.089l-7.165 3.583A.75.75 0 0 1 3.75 21V5.507c0-1.47 1.073-2.756 2.57-2.930Z"/></svg>"##;
const OCULTAR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path d="M3.53 2.47a.75.75 0 0 0-1.06 1.06l18 18a.75.75 0 1 0 1.06-1.06l-18-18ZM22.676 12.553a11.249 11.249 0 0 1-2.631 4.31l-3.099-3.099a5.25 5.25 0 0 0-6.71-6.71L7.759 4.577a11.217 11.217 0 0 1 4.242-.827c4.97 0 9.185 3.223 10.675 7.69.12.362.12.752 0 1.113Z"/><path d="M15.75 12c0 .18-.013.357-.037.53l-4.244-4.243A3.75 3.75 0 0 1 15.75 12ZM12.53 15.713l-4.243-4.244a3.75 3.75 0 0 0 4.244 4.243Z"/><path d="M6.75 12c0-.619.107-1.213.304-1.764l-3.1-3.1a11.25 11.25 0 0 0-2.63 4.31c-.12.362-.12.752 0 1.114 1.489 4.467 5.704 7.69 10.675 7.69 1.5 0 2.933-.294 4.242-.827l-2.477-2.477A5.25 5.25 0 0 1 6.75 12Z"/></svg>"##;

/// Los modos del control de ruido en el orden del segmentado: de menos a más
/// aislamiento, como la maqueta.
// añadido en la 0.2.2
const MODOS_RUIDO: [u8; 3] = [0, 2, 1];

/// El segmentado del HIG (§9): fondo r12 con 3 de relleno, pestañas r10 de
/// 13/600; la activa con la pareja blanca/negra de la isla.
// añadido en la 0.2.2
fn segmentado_ruido(modo: u8) -> PanelElement<'static> {
    let pestana = |m: u8, etiqueta: &'static str| -> PanelElement<'static> {
        let activa = m == modo;
        container(text(etiqueta).size(13).font(negrita()).color(if activa {
            Color::BLACK
        } else {
            tema::TEXTO2
        }))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: activa.then(|| Color::WHITE.into()),
            border: Border {
                radius: 10.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };
    let etiquetas = ["Desactivada", "Ambiente", "Cancelación"];
    container(Row::with_children(
        MODOS_RUIDO
            .iter()
            .zip(etiquetas)
            .map(|(m, e)| pestana(*m, e)),
    ))
    .width(Length::Fill)
    .height(Length::Fixed(BOTON_H))
    .padding(3)
    .style(|_| container::Style {
        background: Some(Color::from_rgb8(0x1c, 0x1c, 0x1e).into()),
        border: Border {
            radius: 12.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

fn icono_columna(b: &Burbuja, lado: f32) -> PanelElement<'static> {
    let fondo = if b.actual {
        b.color
    } else {
        tema::mezclar(isla(), b.color, 0.32)
    };
    let tinta = if b.actual {
        Color::WHITE
    } else {
        tema::mezclar(isla(), Color::WHITE, 0.45)
    };
    let px = lado * 0.55;
    let dentro: PanelElement<'static> = match b.clase {
        Clase::Player => icono_teñido(icono_propio("musica"), px, tinta),
        Clase::Timer => icono_teñido(icono_inline(RELOJ), px, tinta),
        Clase::Recorder => punto(true, tinta, lado * 0.3),
        Clase::Transfer => icono_teñido(icono_inline(DOCUMENTO), px, tinta),
        Clase::Calendar => icono_teñido(icono_inline(CALENDARIO), px, tinta),
        Clase::Monitor => icono_teñido(icono_inline(CHIP), px, tinta),
        Clase::Auriculares => image(FOTOS_BUDS[4].clone()).width(px).height(px).into(),
        Clase::Asistente(_) => punto(true, tinta, lado * 0.3),
    };
    container(dentro)
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: (lado / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// El punto de la grabadora, liso: el rojo ya basta para llamar la atención.
// 0.1.0
fn punto(activa: bool, color: Color, lado: f32) -> PanelElement<'static> {
    let c = if activa { color } else { tema::TEXTO2 };
    container(Space::new())
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .style(move |_| pill(c))
        .into()
}

/// El ecualizador. Barras con perfil propio y desfase entre ellas: en fase se
/// ven como un acordeón, no como sonido.
// 0.1.0
fn ondas(
    nivel: f32,
    activa: bool,
    color: Color,
    alto: f32,
    barras: usize,
    ancho: f32,
) -> PanelElement<'static> {
    // Segundos dentro del minuto, no desde 1970: a 1,7e9 s un `f32` solo
    // distingue saltos de 128 s y las barras se quedaban congeladas.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| (d.as_millis() % 60_000) as f32 / 1000.0);
    let mut r = row![]
        .spacing((ancho * 0.7).max(3.0))
        .align_y(Vertical::Center);
    for i in 0..barras {
        // El seno de un irracional por el índice da un perfil que no se repite
        // dentro de la fila, que es lo que distingue un vúmetro de una valla.
        // Con pocas barras el perfil tiene que ser suave: con el del vúmetro
        // largo, cinco barras salían dos altas y tres puntos.
        let pocas = barras <= 8;
        let variacion = if pocas { 0.45 } else { 0.55 };
        let b = (1.0 - variacion) + variacion * ((i as f32 * 2.399).sin() * 0.5 + 0.5);
        // El ecualizador de la cabecera es un adorno de «esto suena», no un
        // vúmetro: si le hiciera caso al nivel, una canción bajita lo dejaría
        // en cinco puntos.
        let n = if pocas {
            nivel.clamp(0.8, 1.0)
        } else {
            nivel.clamp(0.6, 1.0)
        };
        let h = if activa {
            // Dos senos por barra con frecuencias propias: una onda común
            // avanzando por la fila se lee como un acordeón, y esto como un
            // ecualizador. Las frecuencias salen de la parte fraccionaria de
            // múltiplos irracionales, así que no hay dos barras iguales.
            let fi = i as f32;
            let f1 = 5.0 + 3.0 * (fi * 1.618).fract();
            let f2 = 9.0 + 4.0 * (fi * 2.399).fract();
            let s1 = (t * f1 + fi * 1.3).sin() * 0.5 + 0.5;
            let s2 = (t * f2 + fi * 2.1).sin() * 0.5 + 0.5;
            let onda = 0.3 + 0.7 * (0.6 * s1 + 0.4 * s2);
            (alto * b * n * onda).max(ancho * 1.6)
        } else {
            ancho * 1.6
        };
        r = r.push(
            container(Space::new())
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(h))
                .style(move |_| pill(color)),
        );
    }
    container(r)
        .height(Length::Fixed(alto))
        .align_y(Vertical::Center)
        .into()
}

/// El color del texto rebajado hasta `f`, mezclando con el fondo en vez de con
/// el canal alfa: el teñido de un SVG **descarta el alfa**, así que un icono
/// «al 35 %» se dibujaba blanco puro.
// 0.1.0
fn apagado(f: f32) -> Color {
    tema::mezclar(tema::card(), tema::texto(), f)
}

// 0.1.0
fn pill(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        border: Border {
            radius: tema::R_PILL.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// 0.1.0
fn negrita() -> Font {
    Font {
        weight: Weight::Semibold,
        ..Font::DEFAULT
    }
}
// 0.1.0
fn medio() -> Font {
    Font {
        weight: Weight::Medium,
        ..Font::DEFAULT
    }
}
// 0.1.0
fn gorda() -> Font {
    Font {
        weight: Weight::Bold,
        ..Font::DEFAULT
    }
}

// 0.1.0
fn icono_propio(nombre: &str) -> Option<icono::Icono> {
    match nombre {
        // No hay fichero para estos: son cuatro trazos, y un SVG en memoria
        // ahorra un asset que solo usa esta tarjeta.
        "cola-lista" => icono_inline(COLA_LISTA),
        otro => icono::propio(otro),
    }
}

// 0.1.0
fn icono_inline(svg: &str) -> Option<icono::Icono> {
    Some(icono::desde_svg(svg))
}

// 0.1.0
fn icono_teñido(ic: Option<icono::Icono>, px: f32, color: Color) -> PanelElement<'static> {
    match ic {
        Some(ic) => icono::ver_teñido_propio(&ic, px, color),
        // Un nombre mal escrito deja el hueco, no descuadra la fila.
        None => Space::new()
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .into(),
    }
}

// Heroicons 2.1.5 `16/solid` (BookOS-HIG) donde existe equivalente. Se quedan
// propios ALEATORIO (Heroicons no tiene «shuffle»), CORAZON (el contorno solo
// existe en 20/24).
// Pictogramas que no están en la biblioteca del panel. Trazos sólidos y no
// `stroke`: el teñido de iced sustituye el color de relleno, y un icono
// perfilado saldría hueco.
const COLA_LISTA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M3 4.75a1 1 0 1 0 0-2 1 1 0 0 0 0 2ZM6.25 3a.75.75 0 0 0 0 1.5h7a.75.75 0 0 0 0-1.5h-7ZM6.25 7.25a.75.75 0 0 0 0 1.5h7a.75.75 0 0 0 0-1.5h-7ZM6.25 11.5a.75.75 0 0 0 0 1.5h7a.75.75 0 0 0 0-1.5h-7ZM4 12.25a1 1 0 1 1-2 0 1 1 0 0 1 2 0ZM3 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2Z"/></svg>"##;
const CORAZON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M8 14.1 2.9 9.2A3.4 3.4 0 0 1 8 4.7a3.4 3.4 0 0 1 5.1 4.5zm0-1.9 3.8-3.7a2 2 0 0 0-3-2.6L8 6.9 7.2 5.9a2 2 0 0 0-3 2.6z"/></svg>"##;
const CORAZON_LLENO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M2 6.342a3.375 3.375 0 0 1 6-2.088 3.375 3.375 0 0 1 5.997 2.26c-.063 2.134-1.618 3.76-2.955 4.784a14.437 14.437 0 0 1-2.676 1.61c-.02.01-.038.017-.05.022l-.014.006-.004.002h-.002a.75.75 0 0 1-.592.001h-.002l-.004-.003-.015-.006a5.528 5.528 0 0 1-.232-.107 14.395 14.395 0 0 1-2.535-1.557C3.564 10.22 1.999 8.558 1.999 6.38L2 6.342Z"/></svg>"##;
/// Esfera con agujas, para la píldora y la burbuja del temporizador.
const RELOJ: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path fill-rule="evenodd" d="M1 8a7 7 0 1 1 14 0A7 7 0 0 1 1 8Zm7.75-4.25a.75.75 0 0 0-1.5 0V8c0 .414.336.75.75.75h3.25a.75.75 0 0 0 0-1.5h-2.5v-3.5Z" clip-rule="evenodd"/></svg>"##;
/// Heroicons 2.1.5 `16/solid/document` (BookOS-HIG), sin dibujo propio.
const DOCUMENTO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M2.5 3.5A1.5 1.5 0 0 1 4 2h4.879a1.5 1.5 0 0 1 1.06.44l3.122 3.12a1.5 1.5 0 0 1 .439 1.061V12.5A1.5 1.5 0 0 1 12 14H4a1.5 1.5 0 0 1-1.5-1.5v-9Z"/></svg>"##;
/// Heroicons 2.1.5 `16/solid/calendar` (BookOS-HIG), sin dibujo propio.
const CALENDARIO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path fill-rule="evenodd" d="M4 1.75a.75.75 0 0 1 1.5 0V3h5V1.75a.75.75 0 0 1 1.5 0V3a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2V1.75ZM4.5 6a1 1 0 0 0-1 1v4.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V7a1 1 0 0 0-1-1h-7Z" clip-rule="evenodd"/></svg>"##;
/// Heroicons 2.1.5 `16/solid/cpu-chip` (BookOS-HIG), sin dibujo propio.
/// Las fotos de los Buds3 Pro que enseña Ajustes en la página de los
/// auriculares (BookOS-Settings/src/assets), recortadas y reducidas a 84 px
/// —28 puntos a escala 3— con Lanczos: el pintor de CPU de iced reduce sin
/// filtrar y la de 512 salía con dientes a este tamaño. Son las del 3 Pro para
/// cualquier modelo, que es también lo que hace Ajustes cuando no reconoce el
/// aparato: el contrato de la isla no dice el modelo más que en el nombre.
///
/// Un `Handle` por foto para todo el proceso: `from_bytes` le da un id nuevo
/// en cada llamada, y crearlo en la vista volvía a decodificar el PNG en cada
/// repintado.
pub(crate) static FOTOS_BUDS: std::sync::LazyLock<[image::Handle; 5]> = std::sync::LazyLock::new(|| {
    [
        include_bytes!("../assets/buds/izquierdo.png").as_slice(),
        include_bytes!("../assets/buds/derecho.png").as_slice(),
        include_bytes!("../assets/buds/estuche.png").as_slice(),
        // Los dos juntos, para la cabecera: 96 px, 32 puntos a escala 3.
        include_bytes!("../assets/buds/pareja.png").as_slice(),
        // Y otra vez a 54 px para la píldora y la burbuja: reducir la de 96 a
        // 18 puntos sin filtro la dejaba en una mancha.
        include_bytes!("../assets/buds/pareja-pequena.png").as_slice(),
    ]
    .map(image::Handle::from_bytes)
});
const FOTO_FILA: f32 = 28.0;
/// Heroicons 2.1.5 `16/solid/speaker-wave` (BookOS-HIG).
const VOLUMEN: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M7.557 2.066A.75.75 0 0 1 8 2.75v10.5a.75.75 0 0 1-1.248.56L3.59 11H2a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h1.59l3.162-2.81a.75.75 0 0 1 .805-.124ZM12.95 3.05a.75.75 0 1 0-1.06 1.06 5.5 5.5 0 0 1 0 7.78.75.75 0 1 0 1.06 1.06 7 7 0 0 0 0-9.9Z"/><path d="M10.828 5.172a.75.75 0 1 0-1.06 1.06 2.5 2.5 0 0 1 0 3.536.75.75 0 1 0 1.06 1.06 4 4 0 0 0 0-5.656Z"/></svg>"##;
/// Heroicons 2.1.5 `16/solid` play, pause, backward y forward: los controles
/// del reproductor rellenos, como el resto de botones redondos de la isla.
pub(crate) const PLAY: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M3 3.732a1.5 1.5 0 0 1 2.305-1.265l6.706 4.267a1.5 1.5 0 0 1 0 2.531l-6.706 4.268A1.5 1.5 0 0 1 3 12.267V3.732Z"/></svg>"##;
pub(crate) const PAUSA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M4.5 2a.5.5 0 0 0-.5.5v11a.5.5 0 0 0 .5.5h1a.5.5 0 0 0 .5-.5v-11a.5.5 0 0 0-.5-.5h-1ZM10.5 2a.5.5 0 0 0-.5.5v11a.5.5 0 0 0 .5.5h1a.5.5 0 0 0 .5-.5v-11a.5.5 0 0 0-.5-.5h-1Z"/></svg>"##;
pub(crate) const ANTERIOR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M8.5 4.75a.75.75 0 0 0-1.107-.66l-6 3.25a.75.75 0 0 0 0 1.32l6 3.25a.75.75 0 0 0 1.107-.66V8.988l5.393 2.921A.75.75 0 0 0 15 11.25v-6.5a.75.75 0 0 0-1.107-.66L8.5 7.013V4.75Z"/></svg>"##;
pub(crate) const SIGUIENTE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M2.53 3.956A1 1 0 0 0 1 4.804v6.392a1 1 0 0 0 1.53.848l5.113-3.196c.16-.1.279-.233.357-.383v2.73a1 1 0 0 0 1.53.849l5.113-3.196a1 1 0 0 0 0-1.696L9.53 3.956A1 1 0 0 0 8 4.804v2.731a.992.992 0 0 0-.357-.383L2.53 3.956Z"/></svg>"##;
const CHIP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M6 6v4h4V6H6Z"/><path fill-rule="evenodd" d="M5.75 1a.75.75 0 0 0-.75.75V3a2 2 0 0 0-2 2H1.75a.75.75 0 0 0 0 1.5H3v.75H1.75a.75.75 0 0 0 0 1.5H3v.75H1.75a.75.75 0 0 0 0 1.5H3a2 2 0 0 0 2 2v1.25a.75.75 0 0 0 1.5 0V13h.75v1.25a.75.75 0 0 0 1.5 0V13h.75v1.25a.75.75 0 0 0 1.5 0V13a2 2 0 0 0 2-2h1.25a.75.75 0 0 0 0-1.5H13v-.75h1.25a.75.75 0 0 0 0-1.5H13V6.5h1.25a.75.75 0 0 0 0-1.5H13a2 2 0 0 0-2-2V1.75a.75.75 0 0 0-1.5 0V3h-.75V1.75a.75.75 0 0 0-1.5 0V3H6.5V1.75A.75.75 0 0 0 5.75 1ZM11 4.5a.5.5 0 0 1 .5.5v6a.5.5 0 0 1-.5.5H5a.5.5 0 0 1-.5-.5V5a.5.5 0 0 1 .5-.5h6Z" clip-rule="evenodd"/></svg>"##;
// Heroicons x-mark y check, al trazo de 1,5 del resto en 16 px.
const CRUZ: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>"##;
const VISTO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 8.5l3.2 3.2L13 4.5"/></svg>"##;
const AVISO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round"><path d="M8 3.5v5.2M8 12h0"/></svg>"##;
const STOP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><rect width="10" height="10" x="3" y="3" rx="1.5"/></svg>"##;
const ALEATORIO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M10.6 2.5a.8.8 0 0 1 1.1 0l1.8 1.8a.8.8 0 0 1 0 1.1l-1.8 1.8a.8.8 0 1 1-1.1-1.1l.4-.4h-.7c-.6 0-1 .2-1.4.7L8 7.2 7.1 6l.3-.4c.7-.9 1.6-1.4 2.7-1.4h.7l-.4-.5a.8.8 0 0 1 0-1.1zM2.3 4.2h1.9c1.1 0 2 .5 2.7 1.4l3 3.9c.4.5.8.7 1.4.7h.7l-.4-.4a.8.8 0 1 1 1.1-1.1l1.8 1.8a.8.8 0 0 1 0 1.1l-1.8 1.8a.8.8 0 1 1-1.1-1.1l.4-.4h-.7c-1.1 0-2-.5-2.7-1.4l-3-3.9c-.4-.5-.8-.7-1.4-.7H2.3a.8.8 0 0 1 0-1.6zm0 6.1h1.9c.5 0 1-.2 1.3-.6l.9 1.2c-.6.7-1.4 1-2.2 1H2.3a.8.8 0 1 1 0-1.6z"/></svg>"##;

// ── Portada ──────────────────────────────────────────────────────────────

/// El color de la burbuja de una actividad que no es la principal: el mismo
/// que lleva su píldora.
// 0.1.0
pub fn color_burbuja(e: &Estado) -> Color {
    match e.clase {
        Clase::Recorder => tema::rojo(),
        Clase::Monitor | Clase::Auriculares => tema::TEXTO2,
        Clase::Player | Clase::Timer | Clase::Transfer | Clase::Calendar => tema::acento(),
        Clase::Asistente(_) => tema::roles_paleta(["blue"]).map_or(tema::acento(), |[c]| c),
    }
}

/// Recorta las esquinas de la carátula.
///
/// El recorte se hace sobre los píxeles y no con un `border-radius` del
/// contenedor: iced dibuja la imagen **encima** del fondo del contenedor, así
/// que el radio del borde no la recorta y la carátula salía con las cuatro
/// esquinas cuadradas dentro de una tarjeta redondeada.
// 0.1.0
fn preparar_portada(portada: Option<&Portada>, lado: u32) -> Option<image::Handle> {
    let p = portada?;
    if p.width == 0 || p.height == 0 {
        return None;
    }
    // Se copia porque `crop_imm` exige un búfer con dueño; solo pasa al
    // cambiar de portada, no por latido.
    let img = ::image::RgbaImage::from_raw(p.width, p.height, p.rgba.clone())?;
    // El cuadrado central, reducido aquí con un filtro de verdad: el pintor
    // de CPU de iced reduce sin filtrar, y una portada de 1000 px pintada a
    // 66 salía con dientes. Además dejaba de ser cuadrada y la imagen se
    // encajaba con franjas vacías arriba y abajo.
    let corte = p.width.min(p.height);
    let cuadrada = ::image::imageops::crop_imm(
        &img,
        (p.width - corte) / 2,
        (p.height - corte) / 2,
        corte,
        corte,
    )
    .to_image();
    let lado = lado.min(corte);
    let reducida = ::image::imageops::resize(
        &cuadrada,
        lado,
        lado,
        ::image::imageops::FilterType::Lanczos3,
    );
    let p = Portada {
        rgba: reducida.into_raw(),
        width: lado,
        height: lado,
    };
    let r = redondear(&p);
    Some(image::Handle::from_rgba(r.width, r.height, r.rgba))
}

// 0.1.0
fn redondear(p: &Portada) -> Portada {
    // El mismo 22 % de radio que usa la carátula sin portada, para que las dos
    // tengan la misma curva a cualquier tamaño.
    let radio = p.width.min(p.height) as f32 * 0.22;
    let mut rgba = p.rgba.clone();
    crate::redondear_esquinas(&mut rgba, p.width, p.height, radio, crate::Alfa::Recto);
    Portada {
        rgba,
        width: p.width,
        height: p.height,
    }
}

/// El color que resume la carátula.
///
/// Media de una rejilla de muestras, no de todos los píxeles: 400 muestras
/// dicen lo mismo que 90 000 y cuestan nada. Después se sube la saturación y se
/// lleva a una luminosidad de trabajo, porque la media de una foto tiende al
/// gris y un gris no sirve para teñir nada.
// 0.1.0
pub(crate) fn color_dominante(p: &Portada) -> Color {
    let (w, h) = (p.width, p.height);
    let paso_x = (w / 20).max(1);
    let paso_y = (h / 20).max(1);
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in (0..h).step_by(paso_y as usize) {
        for x in (0..w).step_by(paso_x as usize) {
            let i = ((y * w + x) * 4) as usize;
            if p.rgba[i + 3] < 128 {
                continue;
            }
            r += p.rgba[i] as u64;
            g += p.rgba[i + 1] as u64;
            b += p.rgba[i + 2] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return tema::acento();
    }
    let (r, g, b) = (
        r as f32 / n as f32 / 255.0,
        g as f32 / n as f32 / 255.0,
        b as f32 / n as f32 / 255.0,
    );
    let media = (r + g + b) / 3.0;
    let saturar = |c: f32| (media + (c - media) * 2.1).clamp(0.0, 1.0);
    let (r, g, b) = (saturar(r), saturar(g), saturar(b));
    let l = 0.299 * r + 0.587 * g + 0.114 * b;
    let objetivo = if tema::es_claro() { 0.48 } else { 0.58 };
    let c = Color::from_rgb(r, g, b);
    if l < objetivo {
        tema::mezclar(c, Color::WHITE, (objetivo - l) / (1.0 - l).max(0.01))
    } else {
        tema::mezclar(c, Color::BLACK, (l - objetivo) / l.max(0.01))
    }
}

/// La barra de progreso de las tarjetas abiertas, sobre el surco neutro del
/// sistema de diseño (`--toff`). El resto iba en el mismo color rebajado, y con
/// el acento claro del tema oscuro las dos mitades apenas se distinguían.
// 0.1.0
fn barra(f: f32, color: Color) -> PanelElement<'static> {
    let lleno = (f.clamp(0.0, 1.0) * 1000.0) as u16;
    let vacio = 1000 - lleno.min(1000);
    container(stack![
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(BARRA_GRUESO))
            .style(|_| pill(tema::control_apagado())),
        row![
            container(Space::new())
                .width(Length::FillPortion(lleno.max(1)))
                .height(Length::Fixed(BARRA_GRUESO))
                .style(move |_| pill(color)),
            Space::new().width(Length::FillPortion(vacio.max(1))),
        ],
    ])
    .width(Length::Fill)
    .height(Length::Fixed(BARRA_GRUESO))
    .into()
}

/// «en 10 min», «en 1 h 05», «ahora». Un aviso no necesita segundos: llegan
/// cada minuto y una cuenta atrás por segundos pediría repintar sin parar.
// 0.1.0
fn cuenta_atras(ms: i64) -> String {
    if ms < 60_000 {
        return "ahora".into();
    }
    let min = ms / 60_000;
    if min < 60 {
        format!("en {min} min")
    } else {
        format!("en {} h {:02}", min / 60, min % 60)
    }
}

/// «CPU 23 %  ·  RAM 41 %  ·  62°», las tres primeras: es lo que cabe en la
/// píldora, y el resto está a un clic.
// 0.1.0
fn resumen_metricas(m: &[Metrica]) -> String {
    m.iter()
        .take(3)
        .map(|x| format!("{} {}", x.etiqueta, x.texto))
        .collect::<Vec<_>>()
        .join("  ·  ")
}

/// `01:18`, y con horas cuando las hay.
// 0.1.0
fn formato_tiempo(ms: i64) -> String {
    let signo = if ms < 0 { "−" } else { "" };
    let s = ms.unsigned_abs() / 1000;
    if s >= 3600 {
        format!("{signo}{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{signo}{:02}:{:02}", s / 60, s % 60)
    }
}

/// `1:18`, sin el cero de delante: es como se escriben las duraciones de las
/// canciones y así los dos rótulos de la barra ocupan lo mismo que el hueco.
// 0.1.0
fn formato_corto(ms: i64) -> String {
    let signo = if ms < 0 { "−" } else { "" };
    let s = ms.unsigned_abs() / 1000;
    if s >= 3600 {
        format!("{signo}{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{signo}{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estado(clase: Clase) -> Estado {
        Estado {
            app_id: "com.bookos.test".into(),
            clase,
            activo: true,
            pausado: false,
            titulo: "Título".into(),
            subtitulo: "Autor".into(),
            posicion_ms: 30_000,
            duracion_ms: 120_000,
            restante_ms: 30_000,
            volumen: 70,
            nivel: 0.5,
            aleatorio: false,
            repetir: false,
            anc: None,
            portada: None,
            cola: Vec::new(),
            metricas: Vec::new(),
        }
    }

    #[test]
    fn zenith_solo_cancela_dentro_del_cierre() {
        for fase in [Fase::Escuchando, Fase::Pensando] {
            let mut a = Actividad::nueva(estado(Clase::Asistente(fase)), false);
            let x = MARGEN_SOMBRA + ASIS_COMPACTO_W - ASIS_LADO - ASIS_CERRAR / 2.0;
            let y = MARGEN_SOMBRA + ASIS_COMPACTO_H / 2.0;
            for (xx, yy) in [
                (x, MARGEN_SOMBRA + 1.0),
                (x, MARGEN_SOMBRA + ASIS_COMPACTO_H - 1.0),
                (MARGEN_SOMBRA + ASIS_COMPACTO_W - 1.0, y),
                (MARGEN_SOMBRA + 100.0, y),
            ] {
                // Fuera de la ✕ la píldora termina la frase o aparta a
                // Zenith, pero nunca tira lo que se ha dicho.
                assert_ne!(
                    a.pulsar(xx, yy).map(|a| a.nombre).as_deref(),
                    Some("cancel")
                );
                assert!(!a.desplegada());
            }
            assert_eq!(a.pulsar(x, y).map(|a| a.nombre).as_deref(), Some("cancel"));
        }
    }

    #[test]
    fn zenith_se_despide_con_clic_fuera_y_esc() {
        let despedida = |fase, cancelar| {
            Actividad::nueva(estado(Clase::Asistente(fase)), false)
                .despedir_asistente(cancelar)
                .map(|a| a.nombre)
        };
        assert_eq!(
            despedida(Fase::Escuchando, false).as_deref(),
            Some("minimize")
        );
        assert_eq!(despedida(Fase::Escuchando, true).as_deref(), Some("cancel"));
        assert_eq!(
            despedida(Fase::Pensando, false).as_deref(),
            Some("minimize")
        );
        assert_eq!(despedida(Fase::Pensando, true).as_deref(), Some("cancel"));
        assert_eq!(
            despedida(Fase::Respuesta, false).as_deref(),
            Some("dismiss")
        );
        assert_eq!(despedida(Fase::Minimizado, true), None);
        let mut player = Actividad::nueva(estado(Clase::Player), false);
        assert!(player.despedir_asistente(true).is_none());
    }

    #[test]
    fn la_isla_se_desvanece_al_irse_y_no_atiende_clics() {
        let mut a = Actividad::nueva(estado(Clase::Asistente(Fase::Escuchando)), true);
        assert!(a.salir());
        assert!(a.animando());
        assert!(!a.salida_terminada());
        assert!(
            a.pulsar(MARGEN_SOMBRA + 100.0, MARGEN_SOMBRA + 10.0)
                .is_none()
        );
        std::thread::sleep(tema::D_POPOVER);
        assert!(a.salida_terminada());
        assert!(a.entrada().0 < 0.01);
        // Sin animaciones no hay fundido que esperar.
        assert!(!Actividad::nueva(estado(Clase::Player), false).salir());
    }

    #[test]
    fn zenith_recoloca_el_boton_al_crecer_la_respuesta() {
        for (fase, nombre) in [(Fase::Respuesta, "dismiss"), (Fase::Error, "retry")] {
            let mut e = estado(Clase::Asistente(fase));
            e.subtitulo = "Una línea".into();
            let mut a = Actividad::nueva(e, false);
            let y_corto = a.boton_asistente_y() + BOTON_H / 2.0;
            let alto_corto = a.size().1;
            let mut largo = estado(Clase::Asistente(fase));
            largo.subtitulo = "Un párrafo con acentos: mañana.\n".repeat(8);
            a.actualizar(largo);
            assert!(a.size().1 > alto_corto);
            assert!(a.cuerpo_asistente.ends_with('…'));
            assert!(
                a.pulsar(MARGEN_SOMBRA + ABIERTO_W / 2.0, MARGEN_SOMBRA + y_corto)
                    .is_none()
            );
            let y = MARGEN_SOMBRA + a.boton_asistente_y() + BOTON_H / 2.0;
            for x in [PAD - 1.0, ABIERTO_W - PAD + 1.0] {
                assert!(a.pulsar(MARGEN_SOMBRA + x, y).is_none());
            }
            let x = MARGEN_SOMBRA + ABIERTO_W / 2.0;
            assert!(
                a.pulsar(x, MARGEN_SOMBRA + a.size().1 - MARGEN_SOMBRA * 2.0 - 1.0)
                    .is_none()
            );
            assert_eq!(a.pulsar(x, y).map(|a| a.nombre).as_deref(), Some(nombre));
        }
    }

    #[test]
    fn zenith_permite_cerrar_y_reabrir_una_respuesta_recogida() {
        for fase in [Fase::Respuesta, Fase::Error] {
            let mut a = Actividad::nueva(estado(Clase::Asistente(fase)), false);
            assert!(a.desplegada());
            let x = MARGEN_SOMBRA + ABIERTO_W - PAD - ASIS_CERRAR / 2.0;
            let y = MARGEN_SOMBRA + PAD + ASIS_CERRAR / 2.0;
            assert_eq!(a.pulsar(x, y).map(|a| a.nombre).as_deref(), Some("dismiss"));
            assert!(a.recoger());
            assert!(
                a.pulsar(MARGEN_SOMBRA + 100.0, MARGEN_SOMBRA + ASIS_COMPACTO_H / 2.0)
                    .is_none()
            );
            assert!(a.desplegada());
        }
    }

    #[test]
    fn el_titular_de_zenith_respeta_el_ancho_real_y_el_unicode() {
        for fuente in [medio(), gorda(), Font::DEFAULT] {
            for texto in [
                "WWW mañana: información útil para Íñigo 🦀".repeat(4),
                "Íñigo\nmañana".into(),
                String::new(),
            ] {
                let recorte = recortar_asistente(&texto, 120.0, 14.0, fuente);
                assert!(crate::widget::ancho_con_fuente(&recorte, 14.0, fuente) <= 120.0);
                assert!(!recorte.contains('\n'));
            }
        }
    }

    fn cola(n: usize) -> Vec<ItemCola> {
        (0..n)
            .map(|i| ItemCola {
                id: format!("id{i}"),
                titulo: "t".into(),
                artista: "a".into(),
                duracion_ms: 200_000,
                favorita: false,
                actual: i == 0,
                portada: None,
            })
            .collect()
    }

    #[test]
    fn click_compacto_abre_sin_ordenar_nada_a_la_app() {
        let mut a = Actividad::nueva(estado(Clase::Player), true);
        assert!(
            a.pulsar(MARGEN_SOMBRA + 20.0, MARGEN_SOMBRA + 20.0)
                .is_none()
        );
        assert_eq!(a.vista, Vista::Abierta);
        assert_eq!(a.size().1, PLAYER_H + CAB_EXTRA + MARGEN_SOMBRA * 2.0);
    }

    #[test]
    fn timer_vencido_ofrece_aplazar_y_parar() {
        let mut e = estado(Clase::Timer);
        e.restante_ms = -4_000;
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + TIMER_BOTON_Y + BOTON_H / 2.0;
        let aplazar = a.pulsar(MARGEN_SOMBRA + 70.0, y).unwrap();
        let parar = a.pulsar(MARGEN_SOMBRA + 300.0, y).unwrap();
        assert_eq!(aplazar.nombre, "snooze");
        assert_eq!(parar.nombre, "stop");
    }

    #[test]
    fn zenith_se_sacude_al_fallar_tras_aterrizar_la_tarjeta() {
        let mut a = Actividad::nueva(estado(Clase::Asistente(Fase::Pensando)), true);
        a.actualizar(estado(Clase::Asistente(Fase::Error)));
        // Mientras crece, quieta en horizontal.
        assert_eq!(a.entrada().2, 0.0);
        // f ≈ 0,1: el primer pico. A 0,2 cruza por cero.
        std::thread::sleep(tema::D_ISLA + D_SACUDIDA / 10);
        assert!(a.entrada().2.abs() > 1.0);
        assert!(a.animando());
        std::thread::sleep(D_SACUDIDA);
        assert!(a.entrada().2.abs() < 0.01);
        // Sin animaciones no hay sacudida.
        let mut b = Actividad::nueva(estado(Clase::Asistente(Fase::Pensando)), false);
        b.actualizar(estado(Clase::Asistente(Fase::Error)));
        assert_eq!(b.entrada(), (1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn pasar_a_otra_actividad_desplaza_el_contenido_sin_reiniciar_la_entrada() {
        let vieja = Actividad::nueva(estado(Clase::Player), true);
        // La anterior ya llegó: una isla nueva empezaría en el 60 % y sin alfa.
        std::thread::sleep(tema::D_ISLA);
        assert!(vieja.entrada().0 > 0.99);
        let (w0, _) = vieja.size();
        let mut nueva = Actividad::nueva(estado(Clase::Timer), true);
        let (w1, _) = nueva.size();
        nueva.continuar_de(vieja);
        let (alfa, zoom, _, _) = nueva.entrada();
        assert!(alfa > 0.99 && (zoom - 1.0).abs() < 0.01, "alfa {alfa}, zoom {zoom}");
        // Arranca con la caja de la anterior y el lienzo cabe las dos.
        assert!((nueva.zona().1 - (w0 - MARGEN_SOMBRA * 2.0)).abs() < 2.0);
        assert!(nueva.size().0 >= w0.max(w1));
        assert!(nueva.animando() && nueva.animando_relevo());
        assert!(nueva.pulsar(MARGEN_SOMBRA + 10.0, MARGEN_SOMBRA + 10.0).is_none());
        std::thread::sleep(tema::D_ISLA);
        assert!(nueva.terminar_relevo());
        assert_eq!(nueva.size().0, w1);
        assert_eq!(nueva.zona(), (0.0, w1 - MARGEN_SOMBRA * 2.0));
        // La que se estaba yendo no se hereda: esa es la que termina y deja paso.
        let mut saliente = Actividad::nueva(estado(Clase::Asistente(Fase::Pensando)), true);
        std::thread::sleep(tema::D_ISLA);
        assert!(saliente.salir());
        let mut relevo = Actividad::nueva(estado(Clase::Asistente(Fase::Pensando)), true);
        relevo.continuar_de(saliente);
        assert!(!relevo.animando_relevo());
        // Nace de la barra: lejos de la entrada terminada (alfa 1) aunque
        // con la máquina cargada ya haya avanzado algo.
        assert!(relevo.entrada().0 < 0.5);
        // Sin animaciones, sin relevo: el destino directo.
        let quieta = Actividad::nueva(estado(Clase::Player), false);
        let mut directa = Actividad::nueva(estado(Clase::Timer), false);
        directa.continuar_de(quieta);
        assert!(!directa.animando_relevo());
    }

    #[test]
    fn zenith_se_despliega_sin_parpadear_y_minimizado_nace_en_su_sitio() {
        let mut a = Actividad::nueva(estado(Clase::Asistente(Fase::Pensando)), true);
        std::thread::sleep(tema::D_ISLA);
        a.actualizar(estado(Clase::Asistente(Fase::Respuesta)));
        let (alfa, zoom, _, _) = a.entrada();
        // Llega la respuesta: la tarjeta parte casi entera, no de la nada.
        assert!(alfa >= 0.5 && zoom >= 0.9, "alfa {alfa}, zoom {zoom}");
        std::thread::sleep(tema::D_ISLA);
        a.actualizar(estado(Clase::Asistente(Fase::Minimizado)));
        // La burbuja está en otro sitio: entra desde cero, sin el pulso.
        let (alfa, zoom, _, _) = a.entrada();
        assert!(alfa < 0.2 && zoom < 0.7, "alfa {alfa}, zoom {zoom}");
        assert!(a.cambio.is_none());
    }

    #[test]
    fn la_voz_de_zenith_llega_sin_saltos() {
        let mut a = Actividad::nueva(
            Estado {
                nivel: 0.0,
                ..estado(Clase::Asistente(Fase::Escuchando))
            },
            true,
        );
        a.actualizar(Estado {
            nivel: 1.0,
            ..estado(Clase::Asistente(Fase::Escuchando))
        });
        // Recién llegado aún no ha subido: no es un escalón.
        assert!(a.nivel_suave() < 0.2);
        let antes = a.entrada().1;
        std::thread::sleep(std::time::Duration::from_secs_f32(NIVEL_TAU * 6.0));
        assert!(a.nivel_suave() > 0.99);
        // La píldora crece con la voz.
        assert!(a.entrada().1 > antes);
        // Sin animaciones, el nivel tal cual.
        let mut b = Actividad::nueva(estado(Clase::Asistente(Fase::Escuchando)), false);
        b.actualizar(Estado {
            nivel: 1.0,
            ..estado(Clase::Asistente(Fase::Escuchando))
        });
        assert_eq!(b.nivel_suave(), 1.0);
    }

    #[test]
    fn zenith_pensando_mueve_la_onda() {
        let a = Actividad::nueva(estado(Clase::Asistente(Fase::Pensando)), true);
        std::thread::sleep(tema::D_ISLA);
        assert!(a.animando());
        assert!(a.ondas_pendientes());
    }

    #[test]
    fn movimiento_reducido_no_mantiene_frames_vivos() {
        let a = Actividad::nueva(estado(Clase::Recorder), false);
        assert!(!a.animando());
        assert_eq!(a.entrada(), (1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn la_vista_abierta_no_repinta_el_bitmap_por_las_ondas() {
        let mut a = Actividad::nueva(estado(Clase::Player), true);
        a.desde = Instant::now() - tema::D_ISLA;
        a.ultimo_frame_ondas = Instant::now() - FRAME_ONDAS;
        assert!(a.animando());
        assert!(a.ondas_pendientes());

        a.vista = Vista::Abierta;
        assert!(!a.animando());
        assert!(!a.ondas_pendientes());

        a.vista = Vista::Compacta;
        a.estado.pausado = true;
        assert!(!a.animando());
        assert!(!a.ondas_pendientes());
    }

    /// El volumen va en la fila secundaria: los extremos de la pista dan 0 y
    /// 100, el centro la mitad, y el altavoz no cuenta como pista.
    #[test]
    fn el_volumen_manda_lo_pulsado_y_mueve_el_pulgar() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + SEC_Y + SEC_H / 2.0;
        let mut en = |x: f32| {
            a.pulsar(MARGEN_SOMBRA + x, y)
                .map(|ac| (ac.nombre, ac.valor))
        };
        assert_eq!(en(VOL_X1 - 1.0), Some(("volume".into(), "100".into())));
        assert_eq!(en(VOL_X0 + 1.0), Some(("volume".into(), "0".into())));
        assert_eq!(
            en((VOL_X0 + VOL_X1) / 2.0),
            Some(("volume".into(), "50".into()))
        );
        assert_eq!(en(VOL_X0 - ICONO_FILA), None, "el altavoz no es pista");
        assert_eq!(a.estado.volumen, 50, "el pulgar no espera a la app");
    }

    /// Los cinco controles caen donde se dibujan. Es la prueba que faltaba: las
    /// zonas se calculan a mano y nada las ata a la vista más que las
    /// constantes de geometría.
    #[test]
    fn los_cinco_controles_caen_donde_se_ven() {
        let mut e = estado(Clase::Player);
        e.cola = cola(2);
        e.cola[0].actual = true;
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let centro = ABIERTO_W / 2.0;
        let mut en = |x: f32| {
            a.pulsar(
                MARGEN_SOMBRA + x,
                MARGEN_SOMBRA + CAB_EXTRA + CTRL_Y + CTRL_H / 2.0,
            )
            .map(|ac| ac.nombre)
        };
        assert_eq!(en(centro), Some("play-pause".into()));
        assert_eq!(
            en(centro - CTRL_PLAY / 2.0 - CTRL_SEP - CTRL_LADO / 2.0),
            Some("previous".into())
        );
        assert_eq!(
            en(centro + CTRL_PLAY / 2.0 + CTRL_SEP + CTRL_LADO / 2.0),
            Some("next".into())
        );
        assert_eq!(
            en(centro - CTRL_PLAY / 2.0 - CTRL_SEP * 2.0 - CTRL_LADO - CTRL_PLANO / 2.0),
            Some("shuffle".into())
        );
        assert_eq!(
            en(centro + CTRL_PLAY / 2.0 + CTRL_SEP * 2.0 + CTRL_LADO + CTRL_PLANO / 2.0),
            Some("favorite".into())
        );
    }

    /// Sin cola no se sabe cuál suena: el corazón no está y su hueco no hace
    /// nada.
    #[test]
    fn sin_cola_no_hay_favorita() {
        let mut e = estado(Clase::Player);
        e.cola.clear();
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let x = ABIERTO_W / 2.0 + CTRL_PLAY / 2.0 + CTRL_SEP * 2.0 + CTRL_LADO + CTRL_PLANO / 2.0;
        assert!(
            a.pulsar(
                MARGEN_SOMBRA + x,
                MARGEN_SOMBRA + CAB_EXTRA + CTRL_Y + CTRL_H / 2.0
            )
            .is_none()
        );
    }

    #[test]
    fn el_segmentado_cambia_el_ruido() {
        let mut e = estado(Clase::Auriculares);
        e.anc = Some(0);
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + CAB_EXTRA + AUR_BOTON_Y + BOTON_H / 2.0;
        let tercio = (ABIERTO_W - PAD * 2.0) / 3.0;
        // La activa no manda nada; la del centro es ambiente (2), la última
        // cancelación (1).
        assert!(a.pulsar(MARGEN_SOMBRA + PAD + tercio / 2.0, y).is_none());
        let accion = a
            .pulsar(MARGEN_SOMBRA + PAD + tercio * 2.5, y)
            .expect("cancelación");
        assert_eq!(
            (accion.nombre.as_str(), accion.valor.as_str()),
            ("anc", "1")
        );
        assert_eq!(a.estado.anc, Some(1));
        // La cabecera de los auriculares le pide a budsd que abra Ajustes.
        let accion = a.pulsar(MARGEN_SOMBRA + CAB_APP_X + 10.0, MARGEN_SOMBRA + 20.0);
        assert_eq!(accion.map(|a| a.nombre), Some("open".to_string()));
    }

    #[test]
    fn la_barra_de_progreso_manda_la_fraccion_pulsada() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + BARRA_Y + BARRA_H / 2.0;
        let ini = PAD + TIEMPO_W + 10.0;
        let fin = ABIERTO_W - PAD - TIEMPO_W - 10.0;
        let medio = a.pulsar(MARGEN_SOMBRA + (ini + fin) / 2.0, y).unwrap();
        assert_eq!(medio.nombre, "seek");
        let f: f32 = medio.valor.parse().unwrap();
        assert!((f - 0.5).abs() < 0.02, "fracción {f}");
        // Y en los extremos no se sale de [0,1].
        let izq: f32 = a.pulsar(MARGEN_SOMBRA, y).unwrap().valor.parse().unwrap();
        let der: f32 = a
            .pulsar(MARGEN_SOMBRA + fin, y)
            .unwrap()
            .valor
            .parse()
            .unwrap();
        assert_eq!((izq, der), (0.0, 1.0));
    }

    /// El botón de la cola es un interruptor: la abre y la vuelve a cerrar.
    #[test]
    fn el_boton_de_la_cola_la_abre_y_la_cierra() {
        let mut e = estado(Clase::Player);
        e.cola = cola(3);
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let alto_sin = a.size().1;
        let x = MARGEN_SOMBRA + PAD + 40.0;
        let y = MARGEN_SOMBRA + SEC_Y + SEC_H / 2.0;
        assert!(a.pulsar(x, y).is_none());
        assert_eq!(a.vista, Vista::Cola);
        assert!(a.size().1 > alto_sin, "la tarjeta crece con la cola");
        assert!(a.pulsar(x, y).is_none());
        assert_eq!(a.vista, Vista::Abierta);
        assert_eq!(a.size().1, alto_sin);
    }

    /// La cola crece con las canciones que tiene, hasta el tope.
    #[test]
    fn la_cola_ocupa_lo_que_tiene_que_ocupar() {
        let mut dos = estado(Clase::Player);
        dos.cola = cola(2);
        let mut a = Actividad::nueva(dos, false);
        a.vista = Vista::Cola;

        let mut muchas = estado(Clase::Player);
        muchas.cola = cola(9);
        let mut b = Actividad::nueva(muchas, false);
        b.vista = Vista::Cola;
        assert!(b.size().1 > a.size().1);

        let mut tope = estado(Clase::Player);
        tope.cola = cola(COLA_MAX);
        let mut c = Actividad::nueva(tope, false);
        c.vista = Vista::Cola;
        assert_eq!(
            b.size().1,
            c.size().1,
            "a partir del tope no sigue creciendo"
        );
    }

    #[test]
    fn la_tercera_cancion_de_la_cola_es_la_tercera_fila() {
        let mut e = estado(Clase::Player);
        e.cola = cola(4);
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Cola;
        let y = PLAYER_H + COLA_SEP + COLA_PAD + COLA_CAB_H + COLA_FILA_H * 2.5;
        let accion = a
            .pulsar(MARGEN_SOMBRA + ABIERTO_W / 2.0, MARGEN_SOMBRA + y)
            .unwrap();
        assert_eq!(accion.nombre, "queue-goto");
        assert_eq!(accion.valor, "id2");
        // Y el corazón de esa misma fila es «favorita», no «ir a».
        let corazon = a
            .pulsar(
                MARGEN_SOMBRA + ABIERTO_W - COLA_PAD - 20.0,
                MARGEN_SOMBRA + y,
            )
            .unwrap();
        assert_eq!(corazon.nombre, "favorite");
        assert_eq!(corazon.valor, "id2");
    }

    #[test]
    fn la_cola_empieza_por_la_cancion_actual() {
        let mut e = estado(Clase::Player);
        e.cola = cola(5);
        for item in &mut e.cola {
            item.actual = false;
        }
        e.cola[2].actual = true;
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Cola;
        // La actual (id2) ocupa la primera fila y la siguiente (id3) la segunda.
        let y0 = PLAYER_H + COLA_SEP + COLA_PAD + COLA_CAB_H + COLA_FILA_H * 0.5;
        let y1 = y0 + COLA_FILA_H;
        let primera = a
            .pulsar(MARGEN_SOMBRA + ABIERTO_W / 2.0, MARGEN_SOMBRA + y0)
            .unwrap();
        let segunda = a
            .pulsar(MARGEN_SOMBRA + ABIERTO_W / 2.0, MARGEN_SOMBRA + y1)
            .unwrap();
        assert_eq!(primera.valor, "id2");
        assert_eq!(segunda.valor, "id3");
    }

    /// Pulsar en la cabecera cierra la tarjeta: es lo que queda por descubrir
    /// cuando no hay ningún botón de cerrar dibujado.
    #[test]
    fn la_cabecera_colapsa_la_tarjeta() {
        for clase in [
            Clase::Player,
            Clase::Timer,
            Clase::Recorder,
            Clase::Transfer,
            Clase::Calendar,
            Clase::Monitor,
            Clase::Auriculares,
        ] {
            let mut a = Actividad::nueva(estado(clase), false);
            a.vista = Vista::Abierta;
            assert!(
                a.pulsar(MARGEN_SOMBRA + 40.0, MARGEN_SOMBRA + PAD + CAB_EXTRA + 10.0)
                    .is_none()
            );
            assert_eq!(a.vista, Vista::Compacta, "{clase:?}");
            // Entre «App ›» y ⋯ la franja de arriba también recoge.
            a.vista = Vista::Abierta;
            assert!(
                a.pulsar(MARGEN_SOMBRA + ABIERTO_W - 80.0, MARGEN_SOMBRA + 20.0)
                    .is_none()
            );
            assert_eq!(a.vista, Vista::Compacta, "{clase:?}");
        }
    }

    #[test]
    fn la_cabecera_abre_la_app_y_los_puntos_el_menu() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let accion = a.pulsar(MARGEN_SOMBRA + CAB_APP_X + 10.0, MARGEN_SOMBRA + 20.0);
        assert_eq!(accion.map(|a| a.nombre), Some(ISLA_ABRIR.to_string()));

        let mas = MARGEN_SOMBRA + ABIERTO_W - MAS_DER - MAS_W / 2.0;
        assert!(a.pulsar(mas, MARGEN_SOMBRA + 20.0).is_none());
        assert_eq!(a.vista, Vista::Menu);
        assert!(!a.recogible(), "con el menú abierto no se recoge sola");

        // Fila del medio: mostrar primero. Sin más vivas ya es la primera y
        // la fila no hace nada.
        let fila = |n: f32| MARGEN_SOMBRA + PAD + CAB_EXTRA + MENU_FILA_H * n + 5.0;
        assert!(a.pulsar(MARGEN_SOMBRA + 60.0, fila(1.0)).is_none());
        assert_eq!(a.vista, Vista::Menu);
        let accion = a.pulsar(MARGEN_SOMBRA + 60.0, fila(2.0));
        assert_eq!(accion.map(|a| a.nombre), Some(ISLA_OCULTAR.to_string()));
        assert_eq!(a.vista, Vista::Abierta);
    }

    #[test]
    fn mostrar_primero_solo_si_no_lo_es() {
        let mut a = Actividad::nueva(estado(Clase::Timer), false);
        a.poner_burbujas(vec![
            Burbuja {
                app_id: "otra".into(),
                clase: Clase::Player,
                color: Color::WHITE,
                activa: true,
                actual: false,
            },
            Burbuja {
                app_id: "yo".into(),
                clase: Clase::Timer,
                color: Color::WHITE,
                activa: true,
                actual: true,
            },
        ]);
        a.vista = Vista::Menu;
        let y = MARGEN_SOMBRA + PAD + CAB_EXTRA + MENU_FILA_H + 5.0;
        let accion = a.pulsar(MARGEN_SOMBRA + 60.0, y);
        assert_eq!(accion.map(|a| a.nombre), Some(ISLA_PRIMERO.to_string()));
    }

    #[test]
    fn la_grabadora_pausa_a_la_izquierda_y_detiene_a_la_derecha() {
        let mut a = Actividad::nueva(estado(Clase::Recorder), false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + REC_BOTON_Y + REC_BOTON / 2.0;
        let centro = MARGEN_SOMBRA + ABIERTO_W / 2.0;
        // Los centros de los dos botones, con la separación con que se dibujan.
        let izq = a.pulsar(centro - 14.0 - REC_BOTON / 2.0, y).unwrap();
        let der = a.pulsar(centro + 14.0 + REC_BOTON / 2.0, y).unwrap();
        assert_eq!(
            (izq.nombre.as_str(), der.nombre.as_str()),
            ("pause", "stop")
        );
        a.estado.pausado = true;
        let izq = a.pulsar(centro - 14.0 - REC_BOTON / 2.0, y).unwrap();
        assert_eq!(izq.nombre, "resume");
    }

    #[test]
    fn el_temporizador_en_marcha_pausa_y_el_pausado_reanuda() {
        let mut a = Actividad::nueva(estado(Clase::Timer), false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + TIMER_BOTON_Y + BOTON_H / 2.0;
        let x = MARGEN_SOMBRA + PAD + 40.0;
        assert_eq!(a.pulsar(x, y).unwrap().nombre, "pause");
        a.estado.pausado = true;
        assert_eq!(a.pulsar(x, y).unwrap().nombre, "resume");
    }

    /// Los botones de una sola acción ocupan todo el ancho y responden en
    /// toda su fila; por encima, la tarjeta se recoge.
    #[test]
    fn los_botones_anchos_mandan_su_orden() {
        for (clase, y, nombre) in [
            (Clase::Transfer, TRANSFER_BOTON_Y, "cancel"),
            (Clase::Calendar, CAL_BOTON_Y, "dismiss"),
            (Clase::Auriculares, AUR_BOTON_Y, "open"),
        ] {
            let mut a = Actividad::nueva(estado(clase), false);
            a.vista = Vista::Abierta;
            let accion = a.pulsar(
                MARGEN_SOMBRA + ABIERTO_W - PAD - 5.0,
                MARGEN_SOMBRA + y + BOTON_H / 2.0,
            );
            assert_eq!(accion.map(|a| a.nombre).as_deref(), Some(nombre));
            // Y el botón acaba dentro de la tarjeta.
            assert!(y + BOTON_H + PAD <= a.size().1 - MARGEN_SOMBRA * 2.0);
        }
    }

    /// Un auricular que no informa no puede salir como el más descargado.
    #[test]
    fn un_auricular_sin_lectura_es_desconocido_y_no_cero() {
        let m = |v: f32, critica| Metrica {
            etiqueta: "x".into(),
            valor: v,
            texto: format!("{:.0} %", v * 100.0),
            critica,
        };
        assert!(desconocida(&m(0.0, false)));
        assert!(!desconocida(&m(0.1, true)));
        assert!(!desconocida(&m(0.5, false)));
    }

    /// Una hora larga de grabación no puede leerse como «01:05».
    #[test]
    fn a_partir_de_una_hora_se_enseñan_las_horas() {
        assert_eq!(formato_tiempo(65_000), "01:05");
        assert_eq!(formato_tiempo(3_725_000), "1:02:05");
        assert_eq!(formato_tiempo(-7_000), "−00:07");
        assert_eq!(formato_corto(65_000), "1:05");
        assert_eq!(formato_corto(235_000), "3:55");
    }

    /// El color de la carátula tiene que servir de fondo con texto encima, así
    /// que ni puede salir negro ni puede salir blanco.
    #[test]
    fn el_color_de_la_portada_es_utilizable() {
        for (r, g, b) in [(10u8, 10u8, 12u8), (250, 250, 250), (120, 30, 200)] {
            let p = Portada {
                rgba: std::iter::repeat_n([r, g, b, 255], 64 * 64)
                    .flatten()
                    .collect(),
                width: 64,
                height: 64,
            };
            let c = color_dominante(&p);
            let l = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
            assert!(l > 0.12 && l < 0.92, "luminosidad {l} para {r},{g},{b}");
        }
    }

    /// Recortar las esquinas no puede tocar el centro de la imagen.
    #[test]
    fn el_recorte_solo_se_come_las_esquinas() {
        let p = Portada {
            rgba: std::iter::repeat_n([200u8, 100, 50, 255], 100 * 100)
                .flatten()
                .collect(),
            width: 100,
            height: 100,
        };
        let r = redondear(&p);
        let alfa = |x: usize, y: usize| r.rgba[(y * 100 + x) * 4 + 3];
        assert_eq!(alfa(50, 50), 255, "el centro se mantiene opaco");
        assert_eq!(alfa(0, 0), 0, "la esquina se recorta");
        assert_eq!(alfa(99, 99), 0, "y la de enfrente también");
    }

    #[test]
    fn recoger_solo_actua_con_la_tarjeta_abierta() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        assert!(!a.recoger(), "compacta no hay nada que recoger");
        a.vista = Vista::Cola;
        assert!(a.recoger());
        assert_eq!(a.vista, Vista::Compacta);
    }

    #[test]
    fn el_boton_central_cambia_el_icono_sin_esperar_a_la_app() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let pausado = a.estado.pausado;
        let accion = a.pulsar(
            MARGEN_SOMBRA + ABIERTO_W / 2.0,
            MARGEN_SOMBRA + CTRL_Y + CTRL_H / 2.0,
        );
        assert_eq!(accion.map(|a| a.nombre).as_deref(), Some("play-pause"));
        assert_eq!(a.estado.pausado, !pausado);
    }

    #[test]
    fn la_miniatura_de_la_cola_sobrevive_a_un_latido_sin_portada() {
        let mut e = estado(Clase::Player);
        e.cola = cola(2);
        e.cola[1].portada = Some(Portada {
            rgba: vec![90; 32 * 20 * 4],
            width: 32,
            height: 20,
        });
        let mut a = Actividad::nueva(e.clone(), false);
        assert!(a.miniaturas.contains_key(&e.cola[1].id));
        e.cola[1].portada = None;
        a.actualizar(e.clone());
        assert!(a.miniaturas.contains_key(&e.cola[1].id), "se conserva");
        e.cola.truncate(1);
        a.actualizar(e);
        assert!(a.miniaturas.is_empty(), "y se olvida al salir de la cola");
    }

    fn burbuja(app_id: &str, clase: Clase) -> Burbuja {
        Burbuja {
            app_id: app_id.into(),
            clase,
            color: tema::acento(),
            activa: true,
            actual: false,
        }
    }

    /// Una actividad a la vez: con otras vivas la píldora solo crece lo del
    /// «+N», y en la compacta no hay nada más que pulsar.
    #[test]
    fn la_compacta_es_una_sola_actividad() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        let (sin, _) = a.size();
        a.poner_burbujas(vec![
            Burbuja {
                actual: true,
                ..burbuja("com.bookos.player", Clase::Player)
            },
            burbuja("com.bookos.clock", Clase::Timer),
        ]);
        assert_eq!(a.size().0, sin + CONTADOR_W);
        assert_eq!(a.zona(), (0.0, PLAYER_COMPACTO_W + CONTADOR_W));
        assert_eq!(
            a.burbuja_en(MARGEN_SOMBRA + 10.0, MARGEN_SOMBRA + 10.0),
            None
        );
    }

    /// Abierta, la columna va en su propio hueco (la tarjeta crece lo que mide)
    /// y cada icono lleva a su actividad.
    #[test]
    fn cada_icono_de_la_columna_es_su_actividad() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let (sin, _) = a.size();
        a.poner_burbujas(vec![
            Burbuja {
                actual: true,
                ..burbuja("com.bookos.player", Clase::Player)
            },
            burbuja("com.bookos.voicerecorder", Clase::Recorder),
            burbuja("com.bookos.clock", Clase::Timer),
        ]);
        assert_eq!(a.size().0, sin + COLUMNA, "la columna no pisa la tarjeta");
        let x = MARGEN_SOMBRA + ABIERTO_W + COLUMNA / 2.0;
        let ids: Vec<_> = a
            .columna()
            .iter()
            .map(|(c, _)| a.burbuja_en(x, MARGEN_SOMBRA + c).map(str::to_owned))
            .collect();
        assert_eq!(
            ids,
            [
                Some("com.bookos.player".into()),
                Some("com.bookos.voicerecorder".into()),
                Some("com.bookos.clock".into())
            ]
        );
        assert_eq!(
            a.burbuja_en(MARGEN_SOMBRA + 20.0, MARGEN_SOMBRA + 20.0),
            None,
            "la tarjeta no es columna"
        );
        let lados: Vec<_> = a.columna().iter().map(|c| c.1).collect();
        assert_eq!(
            lados,
            [COLUMNA_ACTUAL, COLUMNA_OTRA, COLUMNA_OTRA],
            "la actual va grande"
        );
    }
}
