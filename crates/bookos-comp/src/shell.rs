//! Puente entre el shell (widgets iced) y el compositor.
//!
//! El shell se dibuja en CPU sobre [`MemoryRenderBuffer`]s, que Smithay sube
//! como texturas y compone con damage tracking igual que cualquier ventana. No
//! hay superficie Wayland, ni cliente, ni proceso aparte: el panel y el dock
//! son elementos más de la escena.
//!
//! # Aislamiento de pánicos
//!
//! Meter el shell dentro del compositor cambia el modelo de fallos: un `unwrap`
//! desafortunado en un widget ya no mata a una applet, mata la sesión entera.
//! Por eso todo lo que entra en el shell pasa por [`ShellHost::guard`], que
//! atrapa el unwind. Si el shell revienta se marca como muerto y **el
//! compositor sigue con las ventanas vivas**: te quedas sin panel ni dock, que
//! es un fallo que se puede mirar y arreglar, no un cierre de sesión.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{Command, Stdio};

use bookos_shell::{Accion, Ancla, Damage, Shell, TeclaPulsada};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

// añadido en la 0.2.0 -- mide también los dibujos provocados por entrada, fuera del bucle de animación.
thread_local! {
    static COSTE_PINTURA: std::cell::Cell<std::time::Duration> = const { std::cell::Cell::new(std::time::Duration::ZERO) };
    static APERTURAS: std::cell::RefCell<Vec<(&'static str, std::time::Instant)>> = const { std::cell::RefCell::new(Vec::new()) };
}

// añadido en la 0.2.0 -- el coste se consume una vez aunque haya varias salidas.
pub fn recoger_medidas(metricas: &mut crate::metricas::Metricas) {
    metricas.pintado(COSTE_PINTURA.with(|coste| coste.replace(std::time::Duration::ZERO)));
    APERTURAS.with(|aperturas| {
        for (nombre, desde) in aperturas.borrow_mut().drain(..) {
            metricas.apertura(nombre, desde);
        }
    });
}

/// Cada OSD tiene una confirmación audible, pero el evento concreto conserva
/// el significado: el de volumen está pensado para juzgar el nivel resultante
/// y el de error no debe sonar como una pulsación aceptada.
///
/// El de volumen no suena aquí: lo pone `multimedia::chasquido`, que es quien
/// mira «Sonido al cambiar el volumen». Sonando también aquí, cada tecla daba
/// dos chasquidos y el interruptor de Ajustes solo apagaba uno de ellos.
// 0.1.0
// modified 0.1.1 -- recoge el proceso del sonido sin dejar zombis
fn reproducir_sonido_osd(icono: &str) {
    let evento = match icono {
        "sin-red" => "dialog-error",
        nombre if nombre.starts_with("volumen") => return,
        nombre if nombre.starts_with("micro") => "audio-volume-change",
        _ => "keyboard-key",
    };
    // modificado en la 0.2.0 -- el tiempo de arranque decide si merece la pena sustituir este proceso.
    let desde = std::time::Instant::now();
    let resultado = Command::new("canberra-gtk-play")
        .args(["-i", evento])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    tracing::debug!(
        evento,
        arranque_ms = desde.elapsed().as_secs_f64() * 1000.0,
        "arranque del sonido del OSD"
    );
    if let Ok(mut hijo) = resultado {
        std::thread::spawn(move || {
            let _ = hijo.wait();
        });
    }
}

/// Cuál de las dos barras del escritorio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Barra {
    Panel,
    Dock,
}

/// Qué parte del shell se pide en [`ShellHost::elements`].
///
/// La isla se separa del resto con la vista de escritorios abierta: su tarjeta
/// desplegada cuelga sobre las miniaturas, que la escena pone delante de todo
/// lo del shell, y quedaba tapada por ellas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapaShell {
    Todo,
    SinIsla,
    SoloIsla,
}

/// Cómo se comporta una barra ante las ventanas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibilidad {
    /// Siempre a la vista, y las ventanas no le pisan el sitio: el área útil
    /// descuenta lo que ocupa.
    Siempre,
    /// Se aparta cuando una ventana llega a donde está, y vuelve cuando se va.
    /// Es el "esquivar ventanas" de KDE. En este modo **no reserva sitio**: las
    /// ventanas pueden ocupar la pantalla entera, que es justo lo que hace que
    /// haya algo que esquivar.
    Esquivando,
}

/// Lo que hay que recordar por barra.
struct EstadoBarra {
    modo: Visibilidad,
    /// Si hay una ventana donde ella está.
    escondida: bool,
    /// Si el cursor está pegado a su borde, pidiéndola. Gana sobre `escondida`:
    /// llevar el ratón al borde la trae aunque haya una ventana debajo.
    pedida: bool,
    /// Cuándo cambió por última vez, para animar la entrada y la salida.
    desde: std::time::Instant,
    /// Dónde estaba cuando empezó la transición actual.
    ///
    /// Sin esto, interrumpir una animación a medias la hacía **saltar**: sacar
    /// el ratón del borde mientras la barra aún estaba entrando la mandaba de
    /// golpe al otro extremo en vez de devolverla desde donde iba.
    desde_fraccion: f32,
}

impl EstadoBarra {
    // 0.1.0
    fn new(modo: Visibilidad) -> Self {
        Self {
            modo,
            escondida: false,
            pedida: false,
            desde: std::time::Instant::now(),
            desde_fraccion: 0.0,
        }
    }

    /// Cuánto de fuera está, de 0 (a la vista) a 1 (escondida del todo).
    ///
    /// Va con la curva de entrada del sistema, la misma que usan las ventanas
    /// al aparecer: una barra que se aparta con otro movimiento distinto del
    /// resto del escritorio se nota en seguida.
    // 0.1.0
    fn fraccion(&self) -> f32 {
        let t = bookos_shell::tema::fraccion(self.desde.elapsed(), OCULTAR);
        let avance = bookos_shell::tema::C_ENTRADA.eval(t);
        let destino = self.destino();
        self.desde_fraccion + (destino - self.desde_fraccion) * avance
    }

    /// Dónde tiene que acabar: 1 fuera, 0 a la vista. Pedida con el cursor
    /// vuelve aunque siga habiendo una ventana debajo.
    // 0.1.0
    fn destino(&self) -> f32 {
        if self.escondida && !self.pedida {
            1.0
        } else {
            0.0
        }
    }

    /// Arranca una transición desde donde esté ahora mismo.
    // 0.1.0
    fn arrancar(&mut self) {
        self.desde_fraccion = self.fraccion();
        self.desde = std::time::Instant::now();
    }

    // 0.1.0
    fn animando(&self) -> bool {
        self.desde.elapsed() < OCULTAR && (self.fraccion() - self.destino()).abs() > 0.001
    }
}

/// Cuánto más se difumina el fondo bajo una emergente a pantalla completa.
///
/// Tres veces el paso del panel. Con 1 —el mismo cristal que la barra— el
/// escritorio seguía reconociéndose detrás de los iconos del launchpad, que es
/// justo lo que no debe pasar cuando lo que hay delante ocupa la pantalla
/// entera. El panel se queda en 1 porque encima suyo hay que leer texto y más
/// difuminado lo deja gris.
const FUERZA_PANTALLA: f32 = 3.0;

/// Hueco entre el panel y las tarjetas que cuelgan de él, en lógicos.
///
/// Ocho es lo que separa una tarjeta flotante de la barra sin que se lea como
/// desconectada de ella. Pegada, la emergente parecía parte del panel.
pub const SEPARACION: f64 = 8.0;

/// Franja del borde de la pantalla que trae de vuelta a una barra apartada,
/// en píxeles físicos.
///
/// Dos píxeles no bastan —hay que clavar el ratón en el borde— y cincuenta
/// hacen que la barra aparezca sola al pasar cerca. Cinco es lo que usa el
/// auto-ocultar de KDE.
const BORDE_SENSIBLE: f64 = 5.0;

/// Lo que tarda una barra en apartarse o volver.
///
/// Es la duración de tarjeta del sistema. Más lento se siente pesado cuando
/// arrastras una ventana contra el borde; más rápido parece un parpadeo.
const OCULTAR: std::time::Duration = bookos_shell::tema::D_TARJETA;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EstadoArrastreDock {
    Pulsado,
    Arrastrando,
    Cancelado,
}

/// La aplicación se identifica por nombre porque otra ventana puede cambiar
/// la lista del dock mientras se mantiene el botón pulsado.
struct ArrastreDock {
    app_id: String,
    exec: String,
    icono: String,
    /// Dónde se pulsó, para distinguir un clic de un arrastre.
    inicio: (f64, f64),
    desfase: (f64, f64),
    origen_x: f32,
    clic: Accion,
    estado: EstadoArrastreDock,
}

pub struct ShellHost {
    shell: Shell,
    panel: Surface,
    dock: Surface,
    // añadido en la 0.2.0 -- los fotogramas de hover y bote solo desplazan texturas pequeñas.
    dock_iconos: Vec<Surface>,
    dock_realces: Vec<Surface>,
    dock_revision: u64,
    /// La superficie emergente abierta (menú, calendario, launchpad). Se crea
    /// al abrirla y se destruye al cerrarla: a diferencia del panel y del dock,
    /// su tamaño lo decide su contenido y cambia de una a otra.
    emergente: Option<Surface>,
    // añadido en la 0.2.0 -- identifica también la textura que sigue visible durante el cierre.
    emergente_mostrada: Option<&'static str>,
    /// Con qué ancla se colocó. Una emergente que se arrastra —la de emojis—
    /// cambia de ancla sin cambiar de tamaño, y hay que volver a colocarla.
    ancla_colocada: Option<Ancla>,
    /// Página anterior del Launchpad mientras cruza con la nueva. Ambos
    /// buffers están ya rasterizados; la GPU solo cambia posición y alfa.
    /// El `f32` es la dirección del paso de 140 px.
    // modificado en la 0.2.0 -- el avance es un muelle, no una curva con duración fija.
    pagina_anterior: Option<(Surface, bookos_shell::tema::Transicion, f32)>,
    /// La vista anterior del launchpad mientras dura una transición suya —abrir
    /// o cerrar una carpeta, o un fundido—. Mismo truco que
    /// [`Estado::pagina_anterior`]: el shell no puede animar el contenido
    /// porque rasterizar la rejilla cuesta 11 ms, así que se guarda lo que
    /// había ya rasterizado y se cruzan las dos superficies en la GPU.
    // modificado en la 0.2.0 -- el avance es un muelle, no una curva con duración fija.
    vista_anterior: Option<(
        Surface,
        bookos_shell::tema::Transicion,
        bookos_shell::TransicionLaunchpad,
    )>,
    /// El realce de lo señalado dentro de la emergente. Va aparte porque mover
    /// el ratón por el launchpad repintaría el buffer entero: medido, 23 ms
    /// cada celda que se cruza. Como superficie propia solo cambia su posición.
    realce: Option<Surface>,
    /// Separado del dock y el launchpad para moverlo sin rasterizar el icono
    /// en cada evento del puntero y componerlo por delante del resto.
    agarrado: Option<Surface>,
    /// Se convierte con la escala de cada salida al cruzar entre monitores.
    agarrado_posicion: Point<f64, Logical>,
    /// La emergente recién creada todavía no ha salido en ningún fotograma.
    ///
    /// Sin esto la entrada arrancaba al crear la superficie, **antes** de
    /// rasterizarla: el launchpad cuesta decenas de milisegundos en pintarse y
    /// la entrada dura 180, así que a veces el primer fotograma que se veía ya
    /// llegaba con la animación acabada y el launchpad aparecía de golpe.
    entrada_sin_ver: std::cell::Cell<bool>,
    /// La emergente se está yendo: sus superficies siguen vivas hasta que
    /// termine la animación de salida. Sin esto desaparecía de golpe, que es lo
    /// que hace que un escritorio parezca hecho a trozos.
    cerrando_en: Option<std::time::Instant>,
    // añadido en la 0.2.0 -- Cell permite arrancar en la primera composición sin consumir tiempo de rasterizado.
    muelle_emergente: std::cell::Cell<bookos_shell::tema::Muelle>,
    launchpad_gesto: Option<crate::gestos::Agarre>,
    // añadido en la 0.2.0 -- la vista de escritorios (Meta+W) sigue a cuatro dedos como el launchpad al pellizco.
    vista_gesto: Option<crate::gestos::Agarre>,
    /// Al cerrar desaparece el modelo de la tarjeta, pero las miniaturas deben
    /// acompañar a su textura hasta que termine de salir.
    miniaturas_vista: Vec<Rectangle<i32, Logical>>,
    /// De dónde venía el realce y cuándo empezó a moverse, para que se deslice
    /// entre celdas en vez de saltar.
    realce_desde: Option<(Point<f64, Physical>, std::time::Instant)>,
    /// Tamaño de la pantalla en píxeles físicos, para colocar el dock.
    screen: (i32, i32),
    /// Conserva el clic hasta distinguir entre activar, ordenar y desanclar.
    arrastre_dock: Option<ArrastreDock>,
    /// Esquina de la burbuja de Zenith minimizado, en lógicos. `None` hasta
    /// que se coloca la primera vez: entonces va a la derecha, a media altura.
    // añadido en la 0.2.0
    burbuja_asistente: Option<(f64, f64)>,
    /// Desde dónde se cogió la burbuja y si ya se ha movido: un clic sin
    /// arrastre la vuelve a abrir.
    // añadido en la 0.2.0
    arrastre_burbuja: Option<ArrastreBurbuja>,
    /// Un arrastre dentro de la emergente ha dejado un repintado pendiente.
    emergente_por_pintar: bool,
    arrastre_pintado_en: std::time::Instant,
    /// Dónde está el puntero, en lógicos. `soltar()` no lo recibe y hace falta
    /// para saber si el icono se sacó del dock o se soltó dentro.
    puntero_en: (f64, f64),
    /// Si el panel está a la vista o esquivando ventanas, y por dónde va su
    /// animación.
    panel_barra: EstadoBarra,
    dock_barra: EstadoBarra,
    /// Capas recortadas del bloqueo, delante del escritorio. Sus posiciones
    /// conservan las coordenadas de la composición a pantalla completa.
    bloqueo: Option<Vec<(bookos_shell::bloqueo::Capa, Surface)>>,
    /// El bloqueo que se acaba de abrir, desvaneciéndose. Sin él, el
    /// escritorio aparecía de golpe en el mismo fotograma en que PAM decía que
    /// sí: se entraba sin transición.
    desbloqueo: Option<(
        Vec<(bookos_shell::bloqueo::Capa, Surface)>,
        std::time::Instant,
    )>,
    /// El velo del color de la canción que va detrás del bloqueo, con el color
    /// con que se pintó. Un degradado vertical en una textura de 1 × `ALTO_VELO`
    /// que la GPU estira a pantalla completa: ver `Bloqueo::velo_medios`.
    bloqueo_velo: Option<(Surface, [u8; 4])>,
    /// El contenido termina con un pintado asentado; la entrada de las capas
    /// se resuelve en la GPU y no necesita ese pintado final.
    bloqueo_contenido_animaba: bool,
    /// La barra de modos de la capa de captura, mientras se elige qué capturar.
    /// Su velo no es una superficie: ver [`Self::velo_captura`].
    captura: Option<Surface>,
    /// La pastilla con las medidas del recuadro, que lo sigue.
    captura_pastilla: Option<Surface>,
    /// La del aviso de volumen y brillo, mientras dura.
    osd: Option<Surface>,
    /// El teclado en pantalla, mientras se ve o se está yendo.
    teclado: Option<Surface>,
    /// El panel de diagnóstico, mientras esté puesto.
    diagnostico: Option<Surface>,
    /// El aviso de la última notificación, mientras dura.
    /// Las superficies de los avisos, por identificador de notificación.
    toasts: Vec<(u32, Surface)>,
    /// Isla superior de música, temporizador o grabación.
    actividad: Option<Surface>,
    /// Se oculta mientras la ventana que la publicó tiene el foco.
    actividad_visible: bool,
    /// Las barras de título, por ventana. Van aquí y no en el shell porque lo
    /// que se guarda es la textura ya subida; el dibujo lo tiene él.
    barras: std::collections::HashMap<u64, Surface>,
    /// La del conmutador de Alt+Tab, mientras el modificador siga pulsado.
    conmutador: Option<Surface>,
    /// En el de ventanas, el recuadro de la elegida y el rótulo con su nombre,
    /// cada uno en su superficie: son lo único que cambia al pasar el cursor, y
    /// así elegir no repinta la tarjeta entera. Ver `Conmutador::animando`.
    conmutador_realce: Option<Surface>,
    conmutador_rotulo: Option<Surface>,
    /// De dónde venía el recuadro del conmutador y cuándo empezó a deslizarse.
    conmutador_desde: Option<(Point<f64, Physical>, std::time::Instant)>,
    /// Una superficie por icono del escritorio. Van detrás de las ventanas, así
    /// que se emiten aparte del resto del shell — ver
    /// [`ShellHost::elementos_escritorio`].
    escritorio: Vec<Surface>,
    /// La banda elástica mientras se arrastra sobre el escritorio, en píxeles
    /// **lógicos** y ya normalizada: `(x, y, ancho, alto)`.
    banda: Option<(f64, f64, f64, f64)>,
    /// Cuando el shell hace panic se desactiva para siempre. Reintentar cada
    /// frame solo llenaría el log y repetiría el fallo.
    dead: bool,
    /// El tema y el acento de antes de pasar de claro a oscuro o al revés,
    /// hasta que la escena les haga la foto del fundido. Ver [`crate::fundido`].
    pub tema_anterior: Option<(bookos_shell::tema::Tema, bookos_shell::tema::Acento)>,
}

struct Surface {
    buffer: MemoryRenderBuffer,
    /// Posición en píxeles **físicos**: es lo que espera el elemento de render.
    location: Point<f64, Physical>,
    /// Tamaño en píxeles **lógicos**: a qué tamaño se dibuja en el escritorio.
    ///
    /// El buffer se rasteriza a resolución física para que se vea nítido, pero
    /// `MemoryRenderBuffer` solo admite escalas de buffer enteras, así que con
    /// escala fraccional (1,75 en este portátil) no puede deducir el tamaño
    /// lógico él solo. Se le pasan las dos cosas al elemento:
    ///
    /// - `src`  = el buffer **entero**, en sus propias coordenadas.
    /// - `size` = este tamaño lógico.
    ///
    /// Pasar solo `size` no vale: `src` toma por defecto el valor de `size`, y
    /// entonces recorta la esquina superior izquierda del buffer y la estira
    /// por 1,75 — que es exactamente el shell gigante y cortado que salía.
    logical: Size<i32, Logical>,
    /// El buffer completo, para usarlo como `src`.
    src: Rectangle<f64, Logical>,
    /// Solo para el realce: si está dibujado con marco de acento. Guardarlo
    /// evita repintarlo cuando lo único que cambia es dónde está.
    marco: bool,
}

/// La burbuja de Zenith cogida con el puntero.
// añadido en la 0.2.0
struct ArrastreBurbuja {
    /// Del puntero a la esquina de la burbuja, para que no salte al cogerla.
    agarre: (f64, f64),
    inicio: (f64, f64),
    movido: bool,
}

/// Separación de la burbuja con el borde al soltarla.
// añadido en la 0.2.0
const MARGEN_BURBUJA: f64 = 24.0;

impl Surface {
    /// Alto del buffer en físicos, que es cuánto tiene que recorrer la barra
    /// para desaparecer por su borde.
    // 0.1.0
    fn buffer_alto(&self) -> f64 {
        self.src.size.h
    }

    // 0.1.0
    fn new(size: (u32, u32), logical: (i32, i32)) -> Self {
        Self {
            // Argb8888, o sea bytes B,G,R,A en little-endian: es lo que hay de
            // verdad en el buffer al salir de iced. Aquí ponía Abgr8888 por el
            // razonamiento de que tiny-skia escribe R,G,B,A, y es falso —
            // medido, el píxel del acento sale [250,143,89,255] y el color es
            // (89,143,250). Se veía en pantalla: el nombre naranja en vez de
            // azul y el zorro de Firefox azul en vez de naranja.
            buffer: MemoryRenderBuffer::new(
                Fourcc::Argb8888,
                (size.0 as i32, size.1 as i32),
                1,
                Transform::Normal,
                None,
            ),
            location: (0.0, 0.0).into(),
            logical: logical.into(),
            src: Rectangle::from_size((size.0 as f64, size.1 as f64).into()),
            marco: false,
        }
    }
}

impl ShellHost {
    /// `width`/`height` son píxeles **físicos** de la pantalla; el shell se
    /// dimensiona en lógicos dividiendo por la escala.
    // 0.1.0
    // modified 0.1.1 -- estado inicial del ancla de emergentes, icono arrastrado, bloqueo y teclado en pantalla
    pub fn new(width: u32, height: u32, scale: f32, config: Option<bookos_shell::Config>) -> Self {
        let logical_width = (width as f32 / scale).round().max(1.0) as u32;
        // Sin configuración se lee del disco, que es lo que hacía siempre: los
        // tests construyen el shell sin pasar nada.
        let config = config.unwrap_or_else(bookos_shell::Config::cargar);
        let modo = |esquiva| {
            if esquiva {
                Visibilidad::Esquivando
            } else {
                Visibilidad::Siempre
            }
        };
        let (modo_panel, modo_dock) = (modo(config.panel_esquiva), modo(config.dock_esquiva));
        let mut shell = Shell::con_config(logical_width, scale, config);
        shell.dock_usar_capas();
        let mut host = Self {
            panel: Surface::new(shell.panel_buffer_size(), shell.panel_logical_size()),
            dock: Surface::new(shell.dock_buffer_size(), shell.dock_logical_size()),
            dock_iconos: Vec::new(),
            dock_realces: Vec::new(),
            dock_revision: u64::MAX,
            shell,
            emergente: None,
            emergente_mostrada: None,
            ancla_colocada: None,
            pagina_anterior: None,
            vista_anterior: None,
            realce: None,
            entrada_sin_ver: std::cell::Cell::new(false),
            cerrando_en: None,
            muelle_emergente: std::cell::Cell::new(bookos_shell::tema::Muelle::nuevo(
                1.0,
                bookos_shell::tema::M_RAPIDO,
            )),
            launchpad_gesto: None,
            vista_gesto: None,
            miniaturas_vista: Vec::new(),
            realce_desde: None,
            screen: (width as i32, height as i32),
            agarrado: None,
            agarrado_posicion: (0.0, 0.0).into(),
            arrastre_dock: None,
            burbuja_asistente: None,
            arrastre_burbuja: None,
            emergente_por_pintar: false,
            arrastre_pintado_en: std::time::Instant::now(),
            puntero_en: (0.0, 0.0),
            panel_barra: EstadoBarra::new(modo_panel),
            dock_barra: EstadoBarra::new(modo_dock),
            bloqueo: None,
            desbloqueo: None,
            bloqueo_velo: None,
            bloqueo_contenido_animaba: false,
            captura: None,
            captura_pastilla: None,
            osd: None,
            teclado: None,
            diagnostico: None,
            toasts: Vec::new(),
            actividad: None,
            actividad_visible: true,
            conmutador: None,
            conmutador_realce: None,
            conmutador_rotulo: None,
            conmutador_desde: None,
            barras: std::collections::HashMap::new(),
            escritorio: Vec::new(),
            banda: None,
            dead: false,
            tema_anterior: None,
        };
        host.place_dock();
        host.escritorio_recolocar();
        host
    }

    /// El dock va centrado abajo. Se recalcula al cambiar el tamaño de pantalla.
    /// Cuántos píxeles lógicos le tiene que reservar el área útil a esta barra.
    ///
    /// Cero si esquiva: en ese modo la barra se aparta sola cuando llega una
    /// ventana, y reservarle sitio dejaría un hueco que nadie puede ocupar y
    /// nada que esquivar.
    ///
    /// El dock **flota**: no llega al borde de la pantalla, así que lo que se
    /// reserva es desde donde empieza el dock hasta abajo —su alto más el aire
    /// que deja—, no solo su alto. Con el alto pelado, una ventana maximizada
    /// se metía en ese hueco y el dock le quedaba encima.
    // 0.1.0
    pub fn reserva(&self, cual: Barra) -> f64 {
        if self.visibilidad(cual) != Visibilidad::Siempre {
            return 0.0;
        }
        match cual {
            Barra::Panel => self.shell.panel_height() as f64,
            Barra::Dock => {
                let (_, y, _, _) = self.dock_rect();
                let alto = self.screen.1 as f64 / self.shell.scale() as f64;
                (alto - y).max(0.0)
            }
        }
    }

    /// Pone el modo de una barra y devuelve el que queda.
    // 0.1.0
    pub fn alternar_visibilidad(&mut self, cual: Barra) -> Visibilidad {
        // Las emergentes del panel están ancladas a sus widgets. Si el panel
        // cambia de modo mientras una sigue abierta, la tarjeta se queda
        // flotando en la franja que este acaba de abandonar.
        if cual == Barra::Panel {
            self.cerrar_emergente();
        }
        let barra = self.barra_mut(cual);
        match barra.modo {
            Visibilidad::Siempre => {
                barra.modo = Visibilidad::Esquivando;
                // El atajo significa también «apártala ahora». Esperar a que
                // una ventana invada la franja hacía que, con una ventana ya
                // maximizada bajo el panel, Meta+Alt+B pareciera no hacer nada.
                barra.escondida = true;
                barra.pedida = false;
                barra.arrancar();
            }
            Visibilidad::Esquivando => {
                barra.modo = Visibilidad::Siempre;
                // Al volver a "siempre visible" se sale del escondite de
                // inmediato: si no, la barra quedaría fuera hasta mover una
                // ventana o llevar el cursor al borde.
                if barra.escondida {
                    barra.arrancar();
                    barra.escondida = false;
                }
            }
        }
        let modo = barra.modo;
        // El modo cambia cuándo se aparta, no su lenguaje visual: el dock se
        // mantiene flotante en ambos casos.
        if cual == Barra::Dock {
            self.place_dock();
        }
        modo
    }

    // 0.1.0
    pub fn visibilidad(&self, cual: Barra) -> Visibilidad {
        self.barra(cual).modo
    }

    /// ¿Cuelga de esta barra una tarjeta abierta ahora mismo?
    ///
    /// Mientras la haya, la barra no puede apartarse: la tarjeta se quedaría
    /// flotando en la franja que la barra acaba de abandonar, o se iría con
    /// ella. Del dock solo cuelga su menú contextual; del panel, cualquiera de
    /// sus emergentes y la isla desplegada: su tarjeta cuelga por debajo del
    /// panel, así que bajar a sus controles sacaba el cursor de la franja de
    /// retención y el panel se apartaba llevándosela.
    // 0.1.0
    // modified 0.1.1 -- el dock no se esconde mientras se arrastra un icono
    fn retenida_por_emergente(&self, cual: Barra) -> bool {
        match cual {
            Barra::Panel => {
                self.shell.hay_emergente()
                    || (self.actividad_visible && self.shell.actividad_abierta())
            }
            Barra::Dock => {
                self.arrastre_dock.is_some() || self.shell.emergente_nombre() == Some("menu-dock")
            }
        }
    }

    /// Le dice a una barra si el cursor la está pidiendo: pegado a su borde de
    /// la pantalla. Devuelve `true` si eso cambia lo que se ve.
    ///
    /// Es lo que hace usable el modo esquivar: la barra se aparta cuando
    /// estorba, pero vuelve en cuanto llevas el ratón al borde, sin tener que
    /// mover la ventana de sitio.
    // 0.1.0
    pub fn reclamada(&mut self, cual: Barra, cursor: (f64, f64)) -> bool {
        let alto = self.screen.1 as f64;
        // Con el menú de un icono abierto, el dock se queda. El menú se dibuja
        // **encima** del dock, así que ir hacia él saca el cursor de la franja
        // de retención y la barra se iba llevándose el menú detrás: clic derecho
        // en un icono y el dock desaparecía antes de poder elegir nada.
        if self.retenida_por_emergente(cual) {
            return false;
        }
        // Hasta dónde llega la barra desde su borde, en físicos: lo que hay que
        // recorrer con el ratón para llegar a sus iconos.
        let limite = match cual {
            Barra::Panel => self.panel.src.size.h,
            Barra::Dock => alto - self.dock.location.y,
        }
        .max(BORDE_SENSIBLE);
        let barra = self.barra_mut(cual);
        if barra.modo != Visibilidad::Esquivando {
            return false;
        }
        // Dos zonas distintas, y esa es la clave de que se pueda usar:
        //
        // - Para **traerla**, la franja del borde de la pantalla: cuando está
        //   apartada, su sitio ya no está bajo el cursor.
        // - Para **mantenerla**, la barra entera. Sin esto era inservible: la
        //   traías al borde, subías el ratón para pulsar un icono y al salir de
        //   los cinco píxeles se volvía a esconder antes de llegar.
        let zona = if barra.pedida { limite } else { BORDE_SENSIBLE };
        let pedida = match cual {
            Barra::Panel => cursor.1 <= zona,
            Barra::Dock => cursor.1 >= alto - zona,
        };
        if barra.pedida == pedida {
            return false;
        }
        barra.arrancar();
        barra.pedida = pedida;
        true
    }

    /// Le dice a una barra si ahora mismo hay una ventana donde ella está.
    ///
    /// Devuelve `true` si eso cambia lo que se ve, o sea si hay que repintar.
    /// En modo "siempre visible" no hace nada: la ventana pasa por debajo.
    // 0.1.0
    pub fn estorbada(&mut self, cual: Barra, hay_ventana: bool) -> bool {
        // Una tarjeta abierta nace de la barra y debe conservarla visible para
        // que el usuario sepa de dónde sale. El modo «esquivar ventanas» no
        // puede ocultar una barra mientras cuelga algo de ella.
        //
        // Se retiene con `pedida` y no tocando `escondida`: esta sigue diciendo
        // la verdad sobre la ventana de debajo, y `pedida` se recalcula con el
        // cursor al cerrar (ver `cerrar_emergente`). Forzando `escondida` la
        // barra se quedaba a la vista tras cerrar la tarjeta hasta que alguna
        // ventana se moviera.
        if self.retenida_por_emergente(cual) {
            let barra = self.barra_mut(cual);
            if barra.modo != Visibilidad::Esquivando {
                return false;
            }
            barra.escondida = hay_ventana;
            if !barra.pedida {
                barra.arrancar();
                barra.pedida = true;
                return true;
            }
            return false;
        }
        let barra = self.barra_mut(cual);
        if barra.modo != Visibilidad::Esquivando || barra.escondida == hay_ventana {
            return false;
        }
        // Con el cursor en el borde el destino no cambia —sigue a la vista—, así
        // que arrancar la transición no la hace parpadear: acaba donde estaba.
        barra.arrancar();
        barra.escondida = hay_ventana;
        true
    }

    /// ¿Sigue moviéndose alguna barra? Mientras sí, hay que repintar.
    // 0.1.0
    pub fn barras_animando(&self) -> bool {
        [&self.panel_barra, &self.dock_barra]
            .iter()
            .any(|b| b.animando())
    }

    // 0.1.0
    fn barra(&self, cual: Barra) -> &EstadoBarra {
        match cual {
            Barra::Panel => &self.panel_barra,
            Barra::Dock => &self.dock_barra,
        }
    }

    // 0.1.0
    fn barra_mut(&mut self, cual: Barra) -> &mut EstadoBarra {
        match cual {
            Barra::Panel => &mut self.panel_barra,
            Barra::Dock => &mut self.dock_barra,
        }
    }

    /// El sitio que ocupa cada barra, en **lógicos**, para saber si una ventana
    /// llega hasta ella. Solo las que esquivan: las fijas no tienen que
    /// enterarse de nada.
    // 0.1.0
    pub fn zonas_barras(
        &self,
    ) -> Vec<(
        Barra,
        smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    )> {
        use smithay::utils::Rectangle;
        if self.dead {
            return Vec::new();
        }
        let escala = self.shell.scale() as f64;
        let mut zonas = Vec::new();
        if self.panel_barra.modo == Visibilidad::Esquivando {
            let (pw, _) = self.shell.panel_buffer_size();
            zonas.push((
                Barra::Panel,
                Rectangle::new(
                    (0, 0).into(),
                    (
                        (pw as f64 / escala).round() as i32,
                        self.shell.panel_height(),
                    )
                        .into(),
                ),
            ));
        }
        if self.dock_barra.modo == Visibilidad::Esquivando {
            let (x, y, w, h) = self.dock_rect();
            zonas.push((
                Barra::Dock,
                Rectangle::new(
                    (x.round() as i32, y.round() as i32).into(),
                    (w.round() as i32, h.round() as i32).into(),
                ),
            ));
        }
        zonas
    }

    /// ¿Está esta barra donde se puede pulsar? Se cuenta como visible mientras
    /// no se haya apartado más de la mitad: durante la animación sigue
    /// respondiendo, que es menos raro que perder el clic a media transición.
    // 0.1.0
    pub fn a_la_vista(&self, cual: Barra) -> bool {
        self.barra(cual).fraccion() < 0.5
    }

    /// Cuánto está desplazada una barra fuera de su sitio, en físicos.
    ///
    /// Positivo es "hacia fuera de la pantalla": el panel se va hacia arriba y
    /// el dock hacia abajo, cada uno por su borde.
    // 0.1.0
    fn desplazamiento(&self, cual: Barra) -> f64 {
        let barra = self.barra(cual);
        let recorrido = match cual {
            Barra::Panel => self.panel.buffer_alto(),
            Barra::Dock => {
                self.dock.buffer_alto()
                    + bookos_shell::DOCK_MARGIN as f64 * self.shell.scale() as f64
            }
        };
        barra.fraccion() as f64 * recorrido
    }

    // 0.1.0
    fn place_dock(&mut self) {
        // Siempre flotante. Antes el modo fijo lo pegaba al canto inferior,
        // eliminaba sus esquinas de abajo y parecía una barra del sistema en
        // vez de un dock. La visibilidad solo decide si puede apartarse.
        if self.guard("dock flotante", |host| host.shell.dock_pegado(false)) == Some(true) {
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }
        let (dw, dh) = self.shell.dock_buffer_size();
        let margin = bookos_shell::DOCK_MARGIN as f64 * self.shell.scale() as f64;
        let x = ((self.screen.0 - dw as i32) as f64 / 2.0).max(0.0);
        let y = (self.screen.1 - dh as i32) as f64 - margin;
        self.dock.location = (x, y.max(0.0)).into();
    }

    // --- Iconos del escritorio ---------------------------------------------

    /// Rehace la rejilla del escritorio y repinta todas sus celdas.
    ///
    /// Se llama al arrancar y al cambiar la pantalla de tamaño o de escala, no
    /// en cada frame: rasterizar una celda cuesta lo suyo y aquí se rasterizan
    /// todas.
    // 0.1.0
    pub fn escritorio_recolocar(&mut self) {
        if self.dead {
            return;
        }
        let escala = self.shell.scale() as f64;
        let logica = (
            self.screen.0 as f32 / escala as f32,
            self.screen.1 as f32 / escala as f32,
        );
        let escala_f32 = self.shell.scale();
        self.guard("recolocar el escritorio", |host| {
            host.shell.escritorio_pantalla(logica, escala_f32);
            let cuantos = host.shell.escritorio().cuantos();
            let buffer = host.shell.escritorio_buffer_size();
            let (lw, lh) = host.shell.escritorio_logical_size();
            host.escritorio = (0..cuantos)
                .map(|_| Surface::new(buffer, (lw as i32, lh as i32)))
                .collect();
            for i in 0..cuantos {
                host.pintar_celda(i);
            }
            host.escritorio_colocar();
        });
    }

    /// Pone cada superficie donde diga la rejilla. Es lo único que hace falta
    /// mientras se arrastra: los buffers ya están pintados y solo cambian de
    /// sitio, que es trabajo de la GPU.
    // 0.1.0
    fn escritorio_colocar(&mut self) {
        let escala = self.shell.scale() as f64;
        for i in 0..self.escritorio.len() {
            let Some((x, y, _, _)) = self.shell.escritorio().rect(i) else {
                continue;
            };
            self.escritorio[i].location = (x as f64 * escala, y as f64 * escala).into();
        }
    }

    // 0.1.0
    fn pintar_celda(&mut self, i: usize) {
        let Self {
            escritorio, shell, ..
        } = self;
        if let Some(surface) = escritorio.get_mut(i) {
            paint(&mut surface.buffer, |buf| shell.draw_escritorio(i, buf));
        }
    }

    /// Cambia la selección y repinta **solo** las celdas que cambiaron de
    /// estado. Es lo que hace que barrer con la banda no cueste la pantalla
    /// entera: por cada movimiento del ratón se rasterizan las una o dos celdas
    /// que el borde acaba de cruzar.
    // 0.1.0
    fn escritorio_cambiar(
        &mut self,
        f: impl FnOnce(&mut bookos_shell::escritorio::Escritorio) -> bool,
    ) -> bool {
        if self.dead {
            return false;
        }
        self.guard("selección del escritorio", |host| {
            let antes = host.shell.escritorio().seleccion();
            if !f(host.shell.escritorio_mut()) {
                return false;
            }
            let despues = host.shell.escritorio().seleccion();
            for i in 0..despues.len() {
                if antes.get(i) != despues.get(i) {
                    host.pintar_celda(i);
                }
            }
            true
        })
        .unwrap_or(false)
    }

    /// Qué icono hay en un punto lógico.
    // 0.1.0
    pub fn escritorio_en(&self, x: f64, y: f64) -> Option<usize> {
        if self.dead {
            return None;
        }
        self.shell.escritorio().en(x as f32, y as f32)
    }

    /// Dónde está el icono `i`, en lógicos. Lo usa el menú del clic derecho
    /// para salir debajo del icono en vez de sobre él.
    // 0.1.0
    pub fn escritorio_rect(&self, i: usize) -> Option<(f32, f32, f32, f32)> {
        if self.dead {
            return None;
        }
        self.shell.escritorio().rect(i)
    }

    // 0.1.0
    pub fn escritorio_seleccionar(&mut self, i: Option<usize>, aditivo: bool) -> bool {
        self.escritorio_cambiar(|e| e.seleccionar(i, aditivo))
    }

    // 0.1.0
    pub fn escritorio_seleccion(&self) -> Vec<bool> {
        if self.dead {
            return Vec::new();
        }
        self.shell.escritorio().seleccion()
    }

    /// Crea una carpeta nueva en el escritorio, del menú de clic derecho.
    ///
    /// Igual que `escritorio_releer`: si la lista cambió hay que rehacer las
    /// superficies de cada icono, porque hay una más que antes.
    // 0.1.0
    // modified 0.2.0 -- también archivos, del submenú «Crear»
    pub fn escritorio_nuevo(
        &mut self,
        tipo: bookos_shell::escritorio::NuevoElemento,
        pantalla: (f32, f32),
    ) -> bool {
        if self.dead {
            return false;
        }
        let cambio = self
            .guard("crear en el escritorio", |host| {
                host.shell.escritorio_nuevo(tipo, pantalla)
            })
            .unwrap_or(false);
        if cambio {
            self.escritorio_recolocar();
        }
        cambio
    }

    /// Manda lo seleccionado del escritorio a la papelera, del mismo menú.
    // 0.1.0
    pub fn escritorio_eliminar_seleccion(&mut self, pantalla: (f32, f32)) -> bool {
        if self.dead {
            return false;
        }
        let cambio = self
            .guard("eliminar del escritorio", |host| {
                host.shell.escritorio_eliminar_seleccion(pantalla)
            })
            .unwrap_or(false);
        if cambio {
            self.escritorio_recolocar();
        }
        cambio
    }

    /// Enseña u oculta los iconos del escritorio. Rehace las superficies: al
    /// ocultarlos no queda ninguna, y al volver hay una por icono.
    // 0.1.1
    pub fn escritorio_mostrar(&mut self, visible: bool, pantalla: (f32, f32)) -> bool {
        if self.dead {
            return false;
        }
        let cambio = self
            .guard("iconos del escritorio", |host| {
                host.shell.escritorio_mostrar(visible, pantalla)
            })
            .unwrap_or(false);
        if cambio {
            self.escritorio_recolocar();
        }
        cambio
    }

    /// Abre el diálogo de color de las carpetas seleccionadas.
    // añadido en la 0.2.0
    pub fn escritorio_elegir_color(&mut self) {
        if self.dead {
            return;
        }
        self.guard("diálogo de color de carpeta", |host| {
            host.shell.abrir_color_carpeta();
        });
        self.sincronizar_emergente();
    }

    /// Pone el color de las carpetas seleccionadas y repinta solo esas.
    // 0.1.1
    pub fn escritorio_colorear(&mut self, color: Option<&str>) -> bool {
        if self.dead {
            return false;
        }
        self.guard("color de carpeta", |host| {
            if !host.shell.escritorio_mut().colorear(color) {
                return false;
            }
            let seleccion = host.shell.escritorio().seleccion();
            for (i, _) in seleccion.iter().enumerate().filter(|(_, s)| **s) {
                host.pintar_celda(i);
            }
            true
        })
        .unwrap_or(false)
    }

    /// Pone la banda elástica —en lógicos y ya normalizada— y marca lo que
    /// cruza. `previa` es la selección de antes de empezar a arrastrar.
    ///
    /// No dice si hay que repintar porque la respuesta es siempre que sí: la
    /// banda cambia de tamaño con cada movimiento, y es parte de la escena
    /// aunque no toque ningún icono.
    // 0.1.0
    pub fn escritorio_banda(&mut self, rect: (f64, f64, f64, f64), previa: &[bool]) {
        self.banda = Some(rect);
        let rect = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        self.escritorio_cambiar(|e| e.banda(rect, previa));
    }

    // 0.1.0
    pub fn escritorio_quitar_banda(&mut self) -> bool {
        self.banda.take().is_some()
    }

    /// Corre los iconos seleccionados mientras se arrastran.
    // 0.1.0
    pub fn escritorio_arrastrar(&mut self, delta: (f64, f64)) {
        if self.dead {
            return;
        }
        self.guard("arrastrar iconos", |host| {
            host.shell
                .escritorio_mut()
                .arrastrar((delta.0 as f32, delta.1 as f32));
            host.escritorio_colocar();
        });
    }

    // 0.1.0
    pub fn escritorio_arrastrando(&self) -> bool {
        !self.dead && self.shell.escritorio().arrastrando()
    }

    /// Deja los iconos en su celda nueva y guarda las posiciones.
    // 0.1.0
    pub fn escritorio_soltar(&mut self) -> bool {
        if self.dead {
            return false;
        }
        self.guard("soltar iconos", |host| {
            let movidos = host.shell.escritorio_mut().soltar();
            host.escritorio_colocar();
            movidos
        })
        .unwrap_or(false)
    }

    // 0.1.0
    pub fn escritorio_abrir(&self, i: usize) -> Option<Accion> {
        if self.dead {
            return None;
        }
        self.shell.escritorio().abrir(i)
    }

    /// Los iconos del escritorio, para dibujarlos **detrás de las ventanas** y
    /// delante del fondo. Por eso no salen por [`ShellHost::elements`], que es
    /// lo que va por encima de todo.
    // 0.1.0
    pub fn elementos_escritorio<R>(
        &self,
        renderer: &mut R,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead {
            return Vec::new();
        }
        self.escritorio
            .iter()
            .filter_map(|surface| {
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    surface.location,
                    &surface.buffer,
                    None,
                    Some(surface.src),
                    Some(surface.logical),
                    Kind::Unspecified,
                )
                .inspect_err(|_| tracing::warn!("no se pudo subir un icono del escritorio"))
                .ok()
            })
            .collect()
    }

    /// La banda que hay puesta, en lógicos, con la escala de la pantalla.
    // 0.1.1
    pub fn banda(&self) -> Option<((f64, f64, f64, f64), f64)> {
        let banda = self
            .banda
            .filter(|&(_, _, w, h)| !self.dead && w >= 1.0 && h >= 1.0)?;
        Some((banda, self.shell.scale() as f64))
    }

    /// La banda elástica con esquinas rectas: relleno y cuatro filos. Es la
    /// reserva para cuando el driver no compila el shader de [`crate::banda`].
    ///
    /// Cinco rectángulos de color y no un buffer propio, por lo mismo que el
    /// velo del launchpad: la banda cambia de tamaño en **cada** movimiento del
    /// ratón, y rasterizarla en CPU sería repintar un buffer de medio millón de
    /// píxeles por evento. Así solo cambia la geometría que se le pasa a la GPU.
    // 0.1.0
    // modified 0.1.1 -- borde de la banda sin esquinas pintadas dos veces
    pub fn banda_elementos(
        &self,
    ) -> Vec<smithay::backend::renderer::element::solid::SolidColorRenderElement> {
        use smithay::backend::renderer::element::Id;
        use smithay::backend::renderer::element::solid::SolidColorRenderElement;

        let Some(((x, y, w, h), escala)) = self.banda() else {
            return Vec::new();
        };
        // Los identificadores tienen que ser los mismos en cada frame: uno
        // nuevo por frame le diría al damage tracker que cambia toda la
        // pantalla, no solo donde estuvo la banda.
        static IDS: std::sync::OnceLock<[Id; 5]> = std::sync::OnceLock::new();
        let ids = IDS.get_or_init(|| std::array::from_fn(|_| Id::new()));

        let acento = bookos_shell::tema::acento();
        let fisico = |v: f64| (v * escala).round() as i32;
        let (x, y, w, h) = (fisico(x), fisico(y), fisico(w), fisico(h));
        // Un píxel lógico de filo: a escala 2 son dos físicos, que es lo que
        // hace que el borde se vea igual de fino en cualquier pantalla.
        let filo = (escala.round().max(1.0) as i32)
            .min(w / 2)
            .min(h / 2)
            .max(1);
        // Los filos verticales van entre los horizontales para que ninguna
        // esquina se pinte dos veces: al 90 % se notaría el cruce.
        let interior = (h - filo * 2).max(1);
        let rectangulos = [
            (
                Rectangle::new(
                    (x + filo, y + filo).into(),
                    ((w - filo * 2).max(1), interior).into(),
                ),
                0.14,
            ),
            (Rectangle::new((x, y).into(), (w, filo).into()), 0.9),
            (
                Rectangle::new((x, y + h - filo).into(), (w, filo).into()),
                0.9,
            ),
            (
                Rectangle::new((x, y + filo).into(), (filo, interior).into()),
                0.9,
            ),
            (
                Rectangle::new((x + w - filo, y + filo).into(), (filo, interior).into()),
                0.9,
            ),
        ];
        ids.iter()
            .zip(rectangulos)
            .map(|(id, (rect, alfa))| {
                SolidColorRenderElement::new(
                    id.clone(),
                    rect,
                    0,
                    // Premultiplicado, que es lo que espera el renderer.
                    [acento.r * alfa, acento.g * alfa, acento.b * alfa, alfa],
                    Kind::Unspecified,
                )
            })
            .collect()
    }

    /// Cuánto falta para que algún widget tenga algo nuevo que enseñar.
    /// `None` = ninguno necesita despertarse solo.
    // 0.1.0
    pub fn next_tick(&self) -> Option<std::time::Duration> {
        if self.dead {
            return None;
        }
        self.shell.next_tick()
    }

    /// Subsistemas de udev que hay que vigilar, según lo que pidan los widgets.
    // 0.1.0
    pub fn subsistemas(&self) -> Vec<&'static str> {
        if self.dead {
            return Vec::new();
        }
        self.shell.subsistemas()
    }

    /// Alto del panel: lo que hay que descontarle al área utilizable por las
    /// ventanas.
    // 0.1.0
    pub fn panel_height(&self) -> i32 {
        if self.dead {
            0
        } else {
            self.shell.panel_height()
        }
    }

    /// El rectángulo lógico de la pantalla principal, que es donde vive **todo**
    /// el shell: panel, dock, emergentes, bloqueo, OSD y avisos.
    ///
    /// Su origen es (0,0) porque `pantallas::normalizar` lleva ahí el de la
    /// principal, y por eso una pantalla colocada a la izquierda o arriba usa
    /// coordenadas negativas. Quien pregunte por un punto tiene que acotarlo
    /// contra esto antes de creerse nada: los puntos que llegan son del
    /// escritorio entero, no de este monitor.
    // 0.1.0
    pub fn area_principal(&self) -> (f64, f64) {
        let escala = self.shell.scale() as f64;
        (self.screen.0 as f64 / escala, self.screen.1 as f64 / escala)
    }

    /// La escala de la pantalla principal. **No** es la de la salida bajo el
    /// puntero: lo que se mide contra las superficies del shell tiene que ir en
    /// la escala en la que están dibujadas, y están dibujadas en la principal.
    // 0.1.0
    pub fn escala_principal(&self) -> f64 {
        self.shell.scale() as f64
    }

    /// ¿Llega el escritorio hasta este punto lógico **global**? Es
    /// [`Self::en_la_principal`] con nombre de lo que pregunta quien la usa: el
    /// escritorio ocupa la principal entera, iconos y hueco vacío incluidos.
    // 0.1.1
    pub fn escritorio_visible(&self) -> bool {
        !self.dead && self.shell.escritorio().visible()
    }

    // 0.1.0
    pub fn escritorio_alcanza(&self, x: f64, y: f64) -> bool {
        !self.dead && self.en_la_principal(x, y)
    }

    /// ¿Cae este punto lógico **global** dentro de la pantalla principal?
    // 0.1.0
    fn en_la_principal(&self, x: f64, y: f64) -> bool {
        let (w, h) = self.area_principal();
        x >= 0.0 && y >= 0.0 && x < w && y < h
    }

    /// El dock en coordenadas **lógicas**: `(x, y, ancho, alto)`.
    ///
    /// `Surface::location` está en físicos porque es lo que quiere el elemento
    /// de render, pero el hit-testing del puntero ocurre en lógicos. La
    /// división se hace aquí, una vez, y no en cada consulta.
    // 0.1.0
    pub fn dock_rect(&self) -> (f64, f64, f64, f64) {
        let scale = self.shell.scale() as f64;
        let (w, h) = self.shell.dock_logical_size();
        (
            self.dock.location.x / scale,
            self.dock.location.y / scale,
            w as f64,
            h as f64,
        )
    }

    /// El rectángulo del icono del dock de una aplicación, en lógicos.
    ///
    /// Es el destino del minimizar: la ventana se encoge hacia **su** icono, no
    /// hacia el centro del dock. Si la aplicación no tiene icono ahí —no está
    /// anclada ni abierta— devuelve `None` y quien llame elegirá otro sitio.
    // 0.1.0
    pub fn icono_dock(&self, app_id: &str) -> Option<(f64, f64, f64, f64)> {
        let i = self
            .shell
            .dock()
            .items()
            .iter()
            .position(|item| bookos_shell::mismo_programa(item.app_id(), app_id))?;
        let (dx, dy, _, dh) = self.dock_rect();
        let centro = self.shell.dock().centro_de(i) as f64;
        // El lado del icono sale del alto del dock menos su margen: pedirle al
        // shell la constante sería exponer una medida de dibujo por un dato que
        // aquí solo sirve para saber a qué tamaño se encoge la ventana.
        let lado = (dh - 20.0).max(24.0);
        Some((dx + centro - lado / 2.0, dy + (dh - lado) / 2.0, lado, lado))
    }

    /// Las zonas que llevan fondo esmerilado, en píxeles **físicos**.
    ///
    /// En físicos porque el desenfoque copia del framebuffer, que es físico, y
    /// convertir dos veces por frame para volver al mismo sitio solo añade
    /// oportunidades de equivocarse con el redondeo.
    // 0.1.0
    // modified 0.1.1 -- las emergentes conservan su cristal sobre pantalla completa
    pub fn zonas_cristal(
        &self,
        completa: bool,
    ) -> Vec<(
        smithay::utils::Rectangle<i32, smithay::utils::Physical>,
        f32,
        f32,
    )> {
        use smithay::utils::Rectangle;
        if self.dead {
            return Vec::new();
        }
        // Con el bloqueo echado no hay cristal: el panel y el dock no se
        // dibujan, pero su fondo esmerilado sí seguía saliendo, y lo que se veía
        // era la silueta redondeada del dock flotando sobre la pantalla de
        // bloqueo sin nada dentro.
        if self.bloqueo.is_some() {
            return Vec::new();
        }
        let escala = self.shell.scale();
        // El launchpad desenfoca la pantalla entera. Va el primero de la lista
        // —o sea, el más al fondo— para que el cristal del panel y el del dock
        // sigan viéndose encima de él.
        if self.shell.emergente_tapa_la_pantalla() {
            return vec![(
                Rectangle::new((0, 0).into(), (self.screen.0, self.screen.1).into()),
                0.0,
                FUERZA_PANTALLA,
            )];
        }
        // La franja de la vista de escritorios **no lleva cristal**. Lo tuvo,
        // y el desenfoque solo cubría la franja: bajo su borde el fondo pasaba
        // de emborronado a nítido en una línea recta, que se veía como un corte
        // en el wallpaper. Lo que la separa de las ventanas de debajo es su
        // propio fondo, más opaco (`Escritorios::fondo`).
        if self.vista_escritorios_visible() {
            return Vec::new();
        }
        // **El panel no lleva cristal.** Lo tuvo, y sobre un fondo con degradado
        // como el de serie el desenfoque no aporta nada: la franja es de 32 px
        // y lo que hay detrás ya es liso, así que solo se notaba en que los
        // bordes del wallpaper se emborronaban al pasar bajo la barra.
        //
        // El dock sí lo conserva: flota sobre las ventanas, y ahí el desenfoque
        // es lo que lo separa de lo que tenga debajo.
        let mut zonas = Vec::with_capacity(2);
        if self.shell.emergente_nombre() == Some("apagar") {
            zonas.push((
                Rectangle::new((0, 0).into(), (self.screen.0, self.screen.1).into()),
                6.0 * escala,
                1.0,
            ));
        }
        // El buscador flota en mitad de la pantalla y ahí debajo hay ventanas:
        // sin cristal, su fondo translúcido deja leer lo que tape. Va antes que
        // el dock para quedar por detrás si llegan a solaparse.
        if self.shell.emergente_usa_cristal()
            && let Some(s) = self.emergente.as_ref()
        {
            let (bw, bh) = (s.src.size.w.round() as i32, s.src.size.h.round() as i32);
            zonas.push((
                Rectangle::new(
                    (s.location.x.round() as i32, s.location.y.round() as i32).into(),
                    (bw, bh).into(),
                ),
                // Este valor es el radio **de las esquinas** del shader,
                // no la intensidad del desenfoque. Usar aquí 6 px dejaba
                // un rectángulo de cristal visible por detrás de las
                // esquinas redondeadas de Spotlight.
                bookos_shell::tema::R_DIALOGO * escala,
                1.0,
            ));
        }
        // La actividad tiene un fondo opaco. Aplicarle aquí una zona de blur
        // rectangular dejaba ver el wallpaper azul en sus esquinas
        // transparentes, por fuera del radio de la tarjeta, como si tuviese un
        // contorno azul. Se volverá a activar cuando el blur admita una máscara
        // redondeada con exactamente la misma geometría de la actividad.
        // El cristal se mueve con su barra. Sin esto, al apartarse el dock se
        // iba el buffer con los iconos y quedaba flotando el rectángulo
        // esmerilado en su sitio — se veía en pantalla.
        let (dw, dh) = self.shell.dock_buffer_size();
        let dock_y = (self.dock.location.y + self.desplazamiento(Barra::Dock)).round() as i32;
        // Pantalla completa oculta las barras, pero las emergentes que se abren
        // encima conservan su cristal. Descartarlo todo dejaba el launchpad
        // transparente y permitía leer la aplicación a través de sus iconos.
        if !completa && dock_y < self.screen.1 {
            zonas.push((
                Rectangle::new(
                    (self.dock.location.x.round() as i32, dock_y).into(),
                    (dw as i32, dh as i32).into(),
                ),
                bookos_shell::tema::R_TARJETA * escala,
                1.0,
            ));
        }
        zonas
    }

    /// ¿Cae el punto lógico dentro de algo del shell? Si es así el clic es
    /// nuestro y no debe llegar a la ventana de debajo.
    ///
    /// El panel ocupa toda la franja de arriba y el dock flota abajo; ambos se
    /// dibujan **por encima** de las ventanas (ver `backend::escena`), así que
    /// tienen que ganar el hit-test.
    /// Con `completa` —una ventana a pantalla completa en la principal— el
    /// panel, el dock y la isla no se dibujan (ver `elements`), así que
    /// tampoco reciben clics. Con las barras en «siempre visible» no hay nada
    /// más que las aparte, y la franja de arriba y el sitio del dock se
    /// quedaban los clics del vídeo: un clic abajo en el centro podía
    /// activar un icono del dock que no se veía.
    // 0.1.0
    // modified 0.1.1 -- el arrastre del dock y el teclado en pantalla se quedan el puntero
    pub fn contiene(&self, x: f64, y: f64, completa: bool) -> bool {
        if self.dead {
            return false;
        }
        // El gesto iniciado en el dock conserva el puntero hasta soltar,
        // incluso al atravesar otra pantalla o cancelar con Escape.
        if self.arrastre_dock.is_some() {
            return true;
        }
        // Lo primero, y no un detalle: el shell **solo existe en la principal**,
        // pero el punto que llega es del escritorio entero. Sin acotar aquí, en
        // una pantalla colocada a la izquierda —x negativa— la franja `y < 32`
        // caía dentro del panel y se tragaba los clics, y peor: con cualquier
        // emergente abierta se devolvía `true` para **toda** la pantalla
        // secundaria, así que el ratón dejaba de funcionar ahí hasta cerrarla.
        // El síntoma era «el ratón se vuelve raro en el monitor de al lado».
        if !self.en_la_principal(x, y) {
            return false;
        }
        // El aviso de una notificación se dibuja encima de las ventanas, así
        // que también gana el hit-test mientras está: si no, pulsarlo escribiría
        // en lo que tenga debajo.
        if self.toast_bajo(x, y).is_some() {
            return true;
        }
        // El teclado tapa lo de debajo también a pantalla completa: se abrió
        // porque esa ventana pidió escribir.
        if self.teclado_en(x, y).is_some() {
            return true;
        }
        if completa {
            return self.shell.hay_emergente();
        }
        if Self::dentro(self.actividad_rect(), x, y).is_some() {
            return true;
        }
        // Una barra apartada no recibe clics: si no, el borde de la pantalla se
        // tragaría pulsaciones destinadas a la ventana que hay debajo.
        if !self.a_la_vista(Barra::Panel)
            && !self.a_la_vista(Barra::Dock)
            && !self.shell.hay_emergente()
        {
            return false;
        }
        if self.a_la_vista(Barra::Panel) && y < self.shell.panel_height() as f64 {
            return true;
        }
        // Con una emergente abierta, el clic es del shell **aunque caiga
        // fuera** de ella: pulsar fuera la cierra, y ese clic no debe llegar
        // además a la ventana de debajo. Es lo que hace cualquier menú.
        if self.shell.hay_emergente() {
            return true;
        }
        // `dock_rect` es su sitio de siempre y no se mueve al apartarse. La
        // guardia de arriba solo corta con las **dos** barras escondidas; con el
        // panel a la vista y el dock fuera, sin esto la franja de abajo seguía
        // siendo del shell y un clic ahí minimizaba la ventana de un icono que
        // no se veía.
        if !self.a_la_vista(Barra::Dock) {
            return false;
        }
        let (dx, dy, dw, dh) = self.dock_rect();
        x >= dx && x < dx + dw && y >= dy && y < dy + dh
    }

    // 0.1.0
    fn dentro(rect: Option<(f64, f64, f64, f64)>, x: f64, y: f64) -> Option<(f32, f32)> {
        let (rx, ry, rw, rh) = rect?;
        (x >= rx && x < rx + rw && y >= ry && y < ry + rh)
            .then_some(((x - rx) as f32, (y - ry) as f32))
    }

    /// Mueve el puntero sobre el shell.
    ///
    /// Fuera del dock se le pasa `None`, para que el icono señalado se apague
    /// al salir. Repintar solo ocurre si el icono señalado **cambia**: mover el
    /// ratón dentro del mismo icono, o por el resto de la pantalla, no cuesta
    /// nada más que esta comparación.
    // 0.1.0
    // modified 0.1.1 -- mueve el icono que se arrastra en el dock
    // modificado en la 0.2.0 -- un arrastre rápido en una tarjeta no atasca el compositor
    pub fn puntero(&mut self, x: f64, y: f64) {
        if self.dead {
            return;
        }
        self.puntero_en = (x, y);
        if self.arrastre_dock.is_some() {
            self.mover_icono_dock(x, y);
            return;
        }
        if self.arrastre_burbuja.is_some() {
            self.arrastrar_burbuja(x, y);
            return;
        }

        if self.shell.hay_emergente() {
            // Con algo agarrado —el deslizador del volumen— el puntero le sigue
            // llegando aunque se salga de la tarjeta: al llevar el volumen al
            // máximo de un tirón la mano se sale, y ahí el deslizador tiene que
            // seguir hasta que se suelte el botón.
            let punto = match (self.emergente_rect(), self.shell.emergente_agarrada()) {
                (Some(r), true) => Some(((x - r.0) as f32, (y - r.1) as f32)),
                (rect, _) => Self::dentro(rect, x, y),
            };
            let repintar = self
                .guard("puntero emergente", |host| {
                    host.shell.emergente_puntero(punto)
                })
                .unwrap_or(false);
            if repintar {
                // Un ratón manda hasta 1000 eventos por segundo y pintar aquí, en
                // el manejador, los encola tras cada rasterizado: el cursor y el
                // color iban cada vez más atrasados. Pinta `animar_emergente`.
                if self.shell.emergente_agarrada() {
                    self.emergente_por_pintar = true;
                } else {
                    self.sincronizar_emergente();
                }
            }
        }

        // Apartado no hay nada que señalar: `dock_rect` es su sitio de siempre y
        // no se mueve con la ocultación, así que sin esto pasar el ratón por la
        // franja de abajo encendía iconos invisibles y rasterizaba el dock una
        // vez por cada uno.
        let punto = self
            .a_la_vista(Barra::Dock)
            .then(|| Self::dentro(Some(self.dock_rect()), x, y))
            .flatten();
        let repintar = self
            .guard("puntero", |host| host.shell.dock_hover(punto))
            .unwrap_or(false);
        if repintar && self.shell.dock_needs_paint() {
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }

        // El resalte del widget bajo el puntero. Con algo a pantalla completa
        // encima —el launchpad— el panel no se ve, y señalar ahí no tiene
        // sentido.
        let en_panel = (self.a_la_vista(Barra::Panel)
            && y < self.shell.panel_height() as f64
            && !self.shell.emergente_tapa_la_pantalla())
        .then_some(x as f32);
        let repintar = self
            .guard("puntero panel", |host| host.shell.panel_puntero(en_panel))
            .unwrap_or(false);
        if repintar {
            self.pintar_panel();
        }
    }

    // 0.1.1
    fn mover_icono_dock(&mut self, x: f64, y: f64) {
        let Some(mut arrastre) = self.arrastre_dock.take() else {
            return;
        };
        let mut repintar = false;
        if arrastre.estado == EstadoArrastreDock::Pulsado
            && (x - arrastre.inicio.0).hypot(y - arrastre.inicio.1)
                > f64::from(bookos_shell::UMBRAL_ARRASTRE)
        {
            arrastre.estado = EstadoArrastreDock::Arrastrando;
            self.agarrado = None;
            self.guard("agarrar icono del dock", |host| {
                host.shell.dock_arrastrar(Some(&arrastre.app_id));
            });
            repintar = true;
        }
        if arrastre.estado == EstadoArrastreDock::Arrastrando
            && let Some((rx, _)) = Self::dentro(Some(self.dock_rect()), x, y)
        {
            repintar |= self
                .guard("ordenar dock", |host| {
                    host.shell.dock_mover(&arrastre.app_id, rx)
                })
                .unwrap_or(false);
        }
        self.arrastre_dock = Some(arrastre);
        if repintar {
            self.guard("pintar orden del dock", |host| {
                paint(&mut host.dock.buffer, |buf| host.shell.draw_dock(buf));
            });
        }
        self.sincronizar_agarrado();
    }

    // 0.1.1
    fn cancelar_arrastre_dock(&mut self) {
        let Some(mut arrastre) = self.arrastre_dock.take() else {
            return;
        };
        if arrastre.estado == EstadoArrastreDock::Arrastrando {
            self.guard("cancelar orden del dock", |host| {
                host.shell.dock_mover(&arrastre.app_id, arrastre.origen_x);
                host.shell.dock_arrastrar(None);
                paint(&mut host.dock.buffer, |buf| host.shell.draw_dock(buf));
            });
            self.agarrado = None;
        }
        // Se conserva hasta soltar para que la aplicación bajo el puntero no
        // reciba una suelta de botón cuya pulsación pertenecía al dock.
        arrastre.estado = EstadoArrastreDock::Cancelado;
        self.arrastre_dock = Some(arrastre);
    }

    // 0.1.1
    pub fn dock_agarrado(&self) -> bool {
        self.arrastre_dock.is_some()
    }

    // 0.1.0
    fn pintar_panel(&mut self) {
        self.guard("pintar panel", |host| {
            let buffer = &mut host.panel.buffer;
            paint(buffer, |buf| host.shell.draw_panel(buf));
        });
    }

    /// Repinta el panel si ha cambiado qué widget tiene su tarjeta abierta.
    // 0.1.0
    fn resaltar_panel(&mut self) {
        if self
            .guard("resalte panel", |host| host.shell.panel_resalte())
            .unwrap_or(false)
        {
            self.pintar_panel();
        }
    }

    // --- Superficies emergentes --------------------------------------------

    /// Crea (o rehace) la superficie de la emergente abierta y la pinta.
    ///
    /// Se llama después de cualquier cosa que pueda abrir, cerrar o cambiar una
    /// emergente. Si no hay ninguna, suelta la superficie: un buffer de un
    /// launchpad a pantalla completa son varios megas que no tiene sentido
    /// guardar mientras está cerrado.
    // 0.1.0
    // modified 0.1.1 -- recoloca la emergente cuando cambia su ancla sin cambiar de tamaño
    // modificado en la 0.2.0 -- la apertura incluye la rasterización inicial.
    fn sincronizar_emergente(&mut self) {
        // añadido en la 0.2.0 -- abrir otra tarjeta interrumpe también el pellizco que retenía la textura anterior.
        if self.shell.hay_emergente() && self.shell.emergente_nombre() != Some("launchpad") {
            self.launchpad_gesto = None;
        }
        self.resaltar_panel();
        // Si el launchpad ha cambiado de vista, lo que hay pintado es el estado
        // de **antes**: se aparta para cruzarlo con el de después. Va lo
        // primero, antes de que nadie repinte encima.
        let transicion = self.shell.launchpad_transicion();
        if let Some(t) = transicion
            && let Some(anterior) = self.emergente.take()
        {
            tracing::debug!(
                ?t,
                "transición de la emergente: se guarda la vista de antes"
            );
            self.vista_anterior = Some((anterior, Self::avance_transicion(t), t));
        }
        let Some(((w, h), ancla, alto_colocacion)) = self.shell.emergente_geometria() else {
            // Cerrándose: las superficies se quedan hasta que acabe la salida.
            if self.cerrando_en.is_none() {
                self.emergente = None;
                self.realce = None;
            }
            return;
        };
        // modificado en la 0.2.0 -- reabrir continúa desde el estado de la salida, también en las tarjetas del panel.
        if self.cerrando_en.take().is_some() {
            let mut muelle = self.muelle_emergente.get();
            muelle.ir_a(1.0);
            self.muelle_emergente.set(muelle);
            self.entrada_sin_ver.set(false);
        }
        let Some(buffer) = self.shell.emergente_buffer_size() else {
            self.emergente = None;
            return;
        };
        self.emergente_mostrada = self.shell.emergente_nombre();

        // La superficie se rehace solo cuando cambia el tamaño: rehacerla en
        // cada repintado tiraría la textura que el compositor ya tiene subida.
        // Cambia al abrir otra distinta y también cuando una crece sobre la
        // marcha —«No molestar» despliega sus duraciones—, y esos dos casos no
        // son lo mismo: al crecer **no** se rearranca la animación de entrada,
        // que haría que la tarjeta volviera a aparecer de golpe con cada clic.
        let nueva = self.emergente.is_none();
        let rehacer = nueva
            || self
                .emergente
                .as_ref()
                .is_some_and(|s| s.logical != (w, h).into());
        if !rehacer && self.ancla_colocada != Some(ancla) {
            self.colocar_emergente(ancla, (w, alto_colocacion.unwrap_or(h)));
        }
        if rehacer {
            self.emergente = Some(Surface::new(buffer, (w, h)));
            // Se coloca por el alto **estable** cuando la emergente tiene uno:
            // el buscador cambia de alto con cada tecla, y usar el de ahora lo
            // hacía saltar media fila mientras escribes.
            self.colocar_emergente(ancla, (w, alto_colocacion.unwrap_or(h)));
            if nueva {
                let mut muelle =
                    bookos_shell::tema::Muelle::nuevo(0.0, bookos_shell::tema::M_RAPIDO);
                muelle.ir_a(1.0);
                self.muelle_emergente.set(muelle);
                self.entrada_sin_ver.set(true);
            }
        }

        if transicion.is_some() {
            // La superficie es nueva, pero la emergente no: no debe sumarle la
            // animación de apertura del launchpad entero a la de la carpeta.
            self.muelle_emergente.set(bookos_shell::tema::Muelle::nuevo(
                1.0,
                bookos_shell::tema::M_RAPIDO,
            ));
        }

        if self.shell.emergente_needs_paint() {
            self.guard("pintar emergente", |host| {
                let Some(surface) = host.emergente.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| host.shell.draw_emergente(buf));
            });
        }

        self.miniaturas_vista = self.miniaturas_vista_sin_animar();
        self.sincronizar_realce();
        self.sincronizar_agarrado();
    }

    /// Coloca —y pinta al empezar— el icono que se lleva la mano.
    ///
    /// Se repinta solo cuando cambia de tamaño, que en la práctica es una vez
    /// por arrastre: moverlo es cambiar `location` y nada más.
    // 0.1.0
    // modified 0.1.1 -- el icono agarrado sigue al puntero también al arrastrar desde el dock
    fn sincronizar_agarrado(&mut self) {
        let (origen, ax, ay, lado) = if let Some(arrastre) = self.arrastre_dock.as_ref()
            && arrastre.estado == EstadoArrastreDock::Arrastrando
        {
            (
                Point::from((0.0, 0.0)),
                (self.puntero_en.0 - arrastre.desfase.0) as f32,
                (self.puntero_en.1 - arrastre.desfase.1) as f32,
                self.shell.dock().tamano_actual() as f32,
            )
        } else {
            let Some((ax, ay, lado)) = self.shell.launchpad_agarrado() else {
                self.agarrado = None;
                return;
            };
            let Some(origen) = self.emergente.as_ref().map(|s| s.location) else {
                self.agarrado = None;
                return;
            };
            (origen, ax, ay, lado)
        };
        let escala = self.shell.scale();
        // Cuadrado al mismo píxel entero que el shell, como el realce: con
        // `(lado * escala).round()` a secas el buffer no coincide con lo que
        // pide `Canvas` a escala fraccionaria y no se pinta nada.
        let logico = bookos_shell::a_pixel_entero(lado, escala);
        let fisico = (logico * escala).round().max(1.0) as u32;
        let rehacer = self
            .agarrado
            .as_ref()
            .is_none_or(|s| s.logical != (logico as i32, logico as i32).into());
        if rehacer {
            self.agarrado = Some(Surface::new(
                (fisico, fisico),
                (logico as i32, logico as i32),
            ));
            self.guard("pintar agarrado", |host| {
                let Some(surface) = host.agarrado.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| {
                    host.shell.draw_agarrado(buf, logico);
                    Vec::new()
                });
            });
        }
        self.agarrado_posicion = (
            origen.x / f64::from(escala) + f64::from(ax),
            origen.y / f64::from(escala) + f64::from(ay),
        )
            .into();
    }

    /// El avance de una transición del launchpad, ya en marcha de 0 a 1.
    ///
    /// Antes eran duraciones fijas con curva (250 ms con rebote al abrir una
    /// carpeta). Con un muelle crítico, abrir y cerrar rápido seguidos no
    /// rearranca nada a tirones y no hay rebote, que en un cambio que no trae
    /// un gesto se leía como que la carpeta no quería quedarse. Cerrar y
    /// fundir usan el rápido: lo que se va no tiene que hacerse esperar.
    // modificado en la 0.2.0 -- muelle en vez de duración y curva.
    fn avance_transicion(t: bookos_shell::TransicionLaunchpad) -> bookos_shell::tema::Transicion {
        use bookos_shell::TransicionLaunchpad as T;
        use bookos_shell::tema;
        let parametros = match t {
            T::AbrirCarpeta | T::AbrirDetalle(_) => tema::M_SUAVE,
            T::CerrarCarpeta | T::Fundido | T::CerrarDetalle(_) => tema::M_RAPIDO,
        };
        let mut avance = tema::Transicion::nueva(0.0, parametros);
        avance.ir_a(1.0);
        avance
    }

    // 0.1.0
    fn geometria_detalle(
        tamano: (f32, f32),
        origen: (f32, f32, f32, f32),
        progreso: f32,
        cerrando: bool,
    ) -> (f32, f32, f32, f32) {
        let t = progreso.clamp(0.0, 1.0);
        let avance = if cerrando { 1.0 - t } else { t };
        let inicial = (origen.2 / tamano.0).min(origen.3 / tamano.1);
        let zoom = inicial + (1.0 - inicial) * avance;
        let x = ((origen.0 + origen.2 / 2.0) - tamano.0 / 2.0) * (1.0 - avance);
        let y = ((origen.1 + origen.3 / 2.0) - tamano.1 / 2.0) * (1.0 - avance);
        ((avance * 1.5).min(1.0), zoom, x, y)
    }

    /// Multiplicador de las transiciones, de `BOOKOS_ANIM`.
    ///
    /// Existe para poder **mirarlas**: a 250 ms una transición o está o no
    /// está, y si no está no se sabe si es que no se dispara o es que se
    /// atraganta. Con `BOOKOS_ANIM=6` dura un segundo y medio y se ve fotograma
    /// a fotograma. Fuera de la variable no cambia nada: sin ella vale 1.
    // 0.1.0
    fn lento() -> f32 {
        std::env::var("BOOKOS_ANIM")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| (1.0..=20.0).contains(v))
            .unwrap_or(1.0)
    }

    /// Coloca —y pinta si hace falta— el realce de lo señalado.
    // 0.1.0
    fn sincronizar_realce(&mut self) {
        let Some((rx, ry, rw, rh, marco)) = self.shell.emergente_realce() else {
            self.realce = None;
            return;
        };
        let Some(origen) = self.emergente.as_ref().map(|s| s.location) else {
            self.realce = None;
            return;
        };
        let escala = self.shell.scale();
        // El tamaño se cuadra con **la misma** función que usa el shell al
        // crear su lienzo, y no con `(rw * escala).round()`.
        //
        // Aquí estaba el fallo por el que el realce del launchpad no se veía a
        // escala fraccionaria, que es la de este portátil. La celda mide 153,6
        // × 115,2 lógicos; esto reservaba (269, 202) físicos, mientras que
        // `Canvas::new` los sube al primer entero lógico que cae en píxel
        // físico entero —156 × 116, o sea (273, 203)— y `draw_realce` moría en
        // cada intento con «buffer del shell con tamaño incoherente». El buffer
        // se quedaba transparente: ni resaltado al pasar el ratón, ni señal de
        // dónde se va a soltar un icono al arrastrarlo, que es lo que hacía
        // parecer que juntar dos iconos en una carpeta no estuviera hecho.
        let logico = (
            bookos_shell::a_pixel_entero(rw, escala),
            bookos_shell::a_pixel_entero(rh, escala),
        );
        let fisico = (
            (logico.0 * escala).round().max(1.0) as u32,
            (logico.1 * escala).round().max(1.0) as u32,
        );

        // Solo se vuelve a pintar si cambia de tamaño o de estilo; recorrer la
        // rejilla con el ratón es mover esta superficie y nada más.
        let rehacer = self.realce.as_ref().is_none_or(|s| {
            s.logical != (logico.0 as i32, logico.1 as i32).into() || s.marco != marco
        });
        if rehacer {
            let mut surface = Surface::new(fisico, (logico.0 as i32, logico.1 as i32));
            surface.marco = marco;
            self.realce = Some(surface);
            self.guard("pintar realce", |host| {
                let Some(surface) = host.realce.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| {
                    host.shell.draw_realce(buf, logico.0, logico.1, marco);
                    Vec::new()
                });
            });
        }

        let destino = Point::<f64, Physical>::from((
            origen.x + (rx * escala) as f64,
            origen.y + (ry * escala) as f64,
        ));
        if let Some(surface) = self.realce.as_mut()
            && surface.location != destino
        {
            // Se guarda de dónde viene para interpolar. Deslizar el realce
            // es gratis —solo cambia una posición, no se repinta nada— y es
            // la diferencia entre que el resaltado siga al cursor o dé
            // saltos de celda en celda.
            let salida = if rehacer { destino } else { surface.location };
            self.realce_desde = Some((salida, std::time::Instant::now()));
            surface.location = destino;
        }
    }

    /// Dónde está el realce **ahora mismo**, contando el deslizamiento.
    // 0.1.0
    fn realce_location(&self, destino: Point<f64, Physical>) -> Point<f64, Physical> {
        let Some((desde, t0)) = self.realce_desde else {
            return destino;
        };
        use bookos_shell::tema;
        // Curva de hover del sistema, la misma que un cambio de fondo al pasar
        // el cursor: es lo que este deslizamiento es.
        let s = tema::C_SUAVE.eval(tema::fraccion(t0.elapsed(), Self::DESLIZ)) as f64;
        (
            desde.x + (destino.x - desde.x) * s,
            desde.y + (destino.y - desde.y) * s,
        )
            .into()
    }

    /// Traduce el anclaje lógico a la posición física de la superficie.
    // modified 0.1.1 -- devuelve al selector de emojis su ancla recortada
    fn colocar_emergente(&mut self, ancla: Ancla, (w, h): (i32, i32)) {
        self.ancla_colocada = Some(ancla);
        let escala = self.shell.scale() as f64;
        let (x, y) = match ancla {
            // Separadas del panel, no pegadas: son tarjetas que flotan sobre
            // el escritorio, y con el borde tocando la barra se leían como una
            // prolongación de ella en vez de como algo aparte.
            Ancla::BajoElPanel { x } => (x as f64, self.shell.panel_height() as f64 + SEPARACION),
            // Centrada bajo el widget que la abrió. Si ese widget no está en el
            // panel —lo han quitado de la configuración— cae al centro, que es
            // mejor que no salir o salir en la esquina.
            Ancla::BajoWidget(nombre) => {
                let centro = self
                    .shell
                    .zonas_panel()
                    .into_iter()
                    .find(|(n, _, _)| *n == nombre)
                    .map(|(_, x0, x1)| ((x0 + x1) / 2.0) as f64)
                    .unwrap_or(self.screen.0 as f64 / escala / 2.0);
                (
                    centro - w as f64 / 2.0,
                    self.shell.panel_height() as f64 + SEPARACION,
                )
            }
            // Sobre el dock y centrada en el icono, sin salirse por los lados.
            Ancla::SobreElDock { x } => {
                let (dx, dy, _, _) = self.dock_rect();
                let ancho = self.screen.0 as f64 / escala;
                let izq = (dx + x as f64 - w as f64 / 2.0)
                    .clamp(SEPARACION, (ancho - w as f64 - SEPARACION).max(SEPARACION));
                (izq, dy - h as f64 - SEPARACION)
            }
            // Centrada en el **área útil**, no en la pantalla: el panel come
            // 32 px arriba y el dock unos 78 abajo, así que centrar contra el
            // rectángulo entero deja la tarjeta por debajo del centro que se
            // percibe. Con algo más alto que el área útil —el launchpad— la
            // cuenta sale negativa y el recorte de abajo la deja en su sitio.
            //
            // Lo que tapa la pantalla —el launchpad— va sin panel, que no se
            // dibuja con él, y con el dock solo si se ve por delante. Centrarlo
            // entre panel y dock lo dejaba en y≈2 a 1440×900: el buscador
            // pegado al borde de arriba.
            Ancla::Centrada => {
                let (arriba, abajo) = if self.shell.emergente_tapa_la_pantalla() {
                    (0.0, self.alto_launchpad())
                } else {
                    (
                        self.shell.panel_height() as f64,
                        self.dock_rect().1 - SEPARACION,
                    )
                };
                let util = (abajo - arriba).max(1.0);
                (
                    (self.screen.0 as f64 / escala - w as f64) / 2.0,
                    arriba + (util - h as f64) / 2.0,
                )
            }
            // Junto al punto donde se hizo clic, sin salirse de la pantalla:
            // el mismo recorte que SobreElDock, pero sin el centrado en un
            // icono porque aquí no hay ninguno.
            Ancla::Punto { x, y } => {
                let (ancho, alto) = (self.screen.0 as f64 / escala, self.screen.1 as f64 / escala);
                (
                    (x as f64).clamp(SEPARACION, (ancho - w as f64 - SEPARACION).max(SEPARACION)),
                    (y as f64).clamp(SEPARACION, (alto - h as f64 - SEPARACION).max(SEPARACION)),
                )
            }
            Ancla::Arriba => (0.0, 0.0),
        };
        // Sin esto, una emergente anclada cerca del borde derecho se saldría de
        // la pantalla y se vería cortada.
        //
        // El recorte de las tarjetas que flotan deja el mismo aire que hay
        // contra el panel, no cero: el calendario cuelga del reloj, que está en
        // la esquina, y pegado al borde exacto se leía como cortado —su sombra
        // cae fuera del buffer y desaparece justo en el lado que toca—. Con
        // margen se ve entero y flotando, que es lo que es.
        //
        // Las que **no** flotan no lo llevan: el launchpad y la franja de
        // escritorios se pegan al borde a propósito, y darles 8 px los
        // descolocaría.
        let margen = match ancla {
            Ancla::Arriba => 0.0,
            _ if self.shell.emergente_tapa_la_pantalla() => 0.0,
            _ => SEPARACION,
        };
        let ancho = self.screen.0 as f64 / escala;
        let alto = self.screen.1 as f64 / escala;
        // Si con margen no cabe, se recorta sin él antes que dejar el mínimo
        // por encima del máximo, que en `clamp` es un pánico.
        let hueco = |disponible: f64, tamano: f64| {
            let max = disponible - tamano - margen;
            if max >= margen {
                (margen, max)
            } else {
                (0.0, (disponible - tamano).max(0.0))
            }
        };
        let (min_x, max_x) = hueco(ancho, w as f64);
        let (min_y, max_y) = hueco(alto, h as f64);
        let (x, y) = (x.clamp(min_x, max_x), y.clamp(min_y, max_y));
        if let Ancla::Punto { x: ax, y: ay } = ancla {
            let recortada = Ancla::Punto {
                x: x as f32,
                y: y as f32,
            };
            if (ax as f64, ay as f64) != (x, y)
                && self.shell.emergente_recortada(x as f32, y as f32)
            {
                self.ancla_colocada = Some(recortada);
            }
        }
        if let Some(surface) = self.emergente.as_mut() {
            surface.location = (x * escala, y * escala).into();
        }
    }

    /// El rectángulo lógico de la emergente abierta.
    // 0.1.0
    fn emergente_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let surface = self.emergente.as_ref()?;
        let escala = self.shell.scale() as f64;
        // modificado en la 0.2.0 -- bajo el buscador que crece, el buffer es transparente: ahí es fuera.
        let alto = self
            .shell
            .emergente_alto_visible()
            .map_or(surface.logical.h as f64, |h| {
                (h as f64).min(surface.logical.h as f64)
            });
        Some((
            surface.location.x / escala,
            surface.location.y / escala,
            surface.logical.w as f64,
            alto,
        ))
    }

    /// Vuelve a preguntar a las barras si el cursor las pide, sin esperar a
    /// que se mueva.
    ///
    /// Con una tarjeta abierta `reclamada` no mira el cursor, así que `pedida`
    /// se quedó como estaba al abrirla —el cursor arriba, pulsando el widget—
    /// aunque luego se bajara a pulsar fuera. Sin volver a mirarlo al cerrarla
    /// el panel no se escondía hasta el siguiente movimiento del ratón.
    // 0.1.0
    fn reclamar_bajo_el_puntero(&mut self) {
        let (ancho, alto) = self.area_principal();
        let escala = self.escala_principal();
        let (x, y) = self.puntero_en;
        if x >= 0.0 && y >= 0.0 && x < ancho && y < alto {
            for cual in [Barra::Panel, Barra::Dock] {
                self.reclamada(cual, (x * escala, y * escala));
            }
        }
    }

    /// ¿Cae el punto dentro de la emergente abierta?
    // 0.2.0
    pub fn emergente_contiene(&self, x: f64, y: f64) -> bool {
        Self::dentro(self.emergente_rect(), x, y).is_some()
    }

    /// Cierra la emergente que hubiera. `true` si había alguna.
    // 0.1.0
    // modified 0.1.1 -- cancela el arrastre del dock
    pub fn cerrar_emergente(&mut self) -> bool {
        self.cancelar_arrastre_dock();
        if self.dead || !self.shell.hay_emergente() {
            return false;
        }
        self.shell.cerrar_emergente();
        self.resaltar_panel();
        self.reclamar_bajo_el_puntero();
        // Las superficies **no** se sueltan aquí: se quedan mientras dura la
        // salida. El realce sí, porque señalar algo que ya se está yendo no
        // significa nada; sin esto, además, se quedaba su recuadro gris
        // flotando en el escritorio para siempre. Y el icono en el aire con
        // más razón: cerrar el launchpad con Esc a media cuesta lo dejaría
        // clavado en mitad del escritorio.
        self.realce = None;
        self.agarrado = None;
        self.realce_desde = None;
        self.pagina_anterior = None;
        self.vista_anterior = None;
        // modificado en la 0.2.0 -- el destino cambia, pero la posición y la velocidad continúan.
        self.launchpad_gesto = None;
        self.entrada_sin_ver.set(false);
        let mut muelle = self.muelle_emergente.get();
        muelle.ir_a(0.0);
        self.muelle_emergente.set(muelle);
        self.cerrando_en = Some(std::time::Instant::now());
        true
    }

    /// Se llama cuando ya no hay animación en curso: suelta lo que quedaba.
    // 0.1.0
    pub fn fin_animacion(&mut self) {
        self.realce_desde = None;
        if self
            .pagina_anterior
            .as_ref()
            .is_some_and(|(_, t, _)| !t.animando())
        {
            self.pagina_anterior = None;
        }
        if self
            .vista_anterior
            .as_ref()
            .is_some_and(|(_, t, _)| !t.animando())
        {
            self.vista_anterior = None;
        }
        self.recoger_cerrada();
        if self
            .desbloqueo
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() >= Self::dura_desbloqueo())
        {
            self.desbloqueo = None;
            self.bloqueo_velo = None;
        }
    }

    /// Suelta lo que quede de una emergente que ya terminó de irse.
    // 0.1.0
    fn recoger_cerrada(&mut self) {
        if self.cerrando_en.is_some() && self.muelle_emergente.get().quieto() {
            self.emergente = None;
            self.cerrando_en = None;
        }
    }

    /// Una tecla para la emergente abierta. `true` si la ha consumido, en cuyo
    /// caso **no** debe llegar al cliente con el foco.
    // 0.1.0
    // modified 0.1.1 -- Esc cancela el arrastre del dock
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> (bool, Option<Accion>) {
        if tecla == TeclaPulsada::Escape && self.arrastre_dock.is_some() {
            self.cancelar_arrastre_dock();
            return (true, None);
        }
        // Esc con Zenith delante lo cancela, salvo que haya una emergente:
        // esa se abrió después y es a la que se le habla.
        if tecla == TeclaPulsada::Escape
            && !self.dead
            && self.actividad_visible
            && !self.shell.hay_emergente()
            && let Some(accion) = self
                .guard("despedir asistente", |host| {
                    host.shell.despedir_asistente(true)
                })
                .flatten()
        {
            return (true, Some(Accion::Actividad(accion)));
        }
        if self.dead || !self.shell.hay_emergente() {
            return (false, None);
        }
        let (consumida, accion) = self
            .guard("tecla", |host| host.shell.emergente_tecla(tecla))
            .unwrap_or((false, None));
        self.sincronizar_emergente();
        (consumida, accion)
    }

    /// Abre el launchpad, o lo cierra si ya lo estaba.
    /// Abre «Acerca de este PC». Cierra lo que hubiera: se llega desde el menú
    /// del logo, que es una emergente él mismo.
    /// Abre la tarjeta que le corresponde a un widget del panel, por su nombre.
    ///
    /// Lo pide el centro de control: sus tarjetas de Wi-Fi y Bluetooth abren
    /// las listas que ya existen en vez de dibujarlas otra vez dentro.
    // 0.1.0
    pub fn abrir_de_widget(&mut self, widget: &str) {
        if self.dead {
            return;
        }
        self.guard("abrir tarjeta", |host| {
            host.shell.abrir_de_widget(widget);
        });
        self.sincronizar_emergente();
    }

    /// Pasa de la tarjeta abierta a otra, como una barra de menús: la de
    /// ahora se quita y la nueva entra con su animación. Es lo que hace el clic
    /// en otro widget del panel y Meta+Z → Meta+X.
    ///
    /// Antes la nueva esperaba a que la vieja terminara de salir. Medido con
    /// el integrador del muelle: la vieja es invisible (alfa < 2 %) a los
    /// 200 ms, pero el muelle no queda quieto hasta los 392, y la nueva aún
    /// tardaba ~130 ms en verse: medio segundo por cambio. Las dos no pueden
    /// convivir porque comparten superficie.
    // 0.1.0
    // modificado en la 0.2.0 -- la nueva no espera a la salida de la anterior
    pub fn cambiar_a_widget(&mut self, widget: &'static str) {
        self.cerrar_emergente();
        self.emergente = None;
        self.cerrando_en = None;
        self.guard("abrir siguiente tarjeta", |host| {
            host.shell.abrir_emergente_nombre(widget)
        });
        self.sincronizar_emergente();
    }

    // 0.1.0
    pub fn confirmar_energia(&mut self, accion: bookos_shell::confirmacion::Energia) {
        self.guard("confirmar energía", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::confirmar_energia(accion));
        });
        self.sincronizar_emergente();
    }

    /// Abre el permiso de compartir pantalla que pide el portal.
    ///
    /// Cierra lo que hubiera abierto: es un diálogo y tiene que verse.
    // 0.1.0
    pub fn abrir_compartir(
        &mut self,
        sesion: u32,
        app: String,
        pantallas: Vec<bookos_shell::PantallaCompartible>,
    ) {
        if self.dead {
            return;
        }
        self.guard("permiso de compartir pantalla", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::compartir(sesion, app, pantallas));
        });
        self.sincronizar_emergente();
    }

    /// Abre la caja para responder a una notificación. `false` si ya no existe.
    // 0.1.0
    pub fn abrir_respuesta(&mut self, id: u32) -> bool {
        if self.dead {
            return false;
        }
        let abierta = self
            .guard("abrir respuesta", |host| host.shell.abrir_respuesta(id))
            .unwrap_or(false);
        self.sincronizar_emergente();
        abierta
    }

    // 0.1.0
    pub fn abrir_acerca(&mut self) {
        if self.dead {
            return;
        }
        self.guard("acerca", |host| {
            host.shell.abrir(bookos_shell::Emergente::acerca());
        });
        self.sincronizar_emergente();
    }

    // añadido en la 0.2.0
    pub fn abrir_atajos(&mut self) {
        if self.dead {
            return;
        }
        self.guard("atajos", |host| {
            host.shell.abrir(bookos_shell::Emergente::atajos());
        });
        self.sincronizar_emergente();
    }

    // 0.1.0
    pub fn alternar_proyeccion(&mut self, conectadas: usize) {
        if self.dead {
            return;
        }
        if self.shell.emergente_nombre() == Some("proyeccion") {
            self.cerrar_emergente();
            return;
        }
        self.guard("selector de proyección", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::proyeccion(conectadas));
        });
        self.sincronizar_emergente();
    }

    // 0.1.0
    pub fn alternar_launchpad(&mut self) {
        if self.dead {
            return;
        }
        if self.shell.hay_emergente() {
            self.cerrar_emergente();
            return;
        }
        // Leer los `.desktop` puede tardar unos milisegundos, así que va dentro
        // del guard: un fichero raro no puede tumbar la sesión.
        let pantalla = (
            self.screen.0 as f32 / self.shell.scale(),
            self.alto_launchpad() as f32,
        );
        self.guard("abrir launchpad", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::launchpad(pantalla));
        });
        // Después de sincronizar: la zona va en coordenadas del launchpad y
        // hasta que su superficie no está colocada no se sabe dónde empieza.
        self.sincronizar_emergente();
        self.avisar_zona_dock();
    }

    // añadido en la 0.2.0 -- el apoyo frena la animación que se está viendo, incluida su salida.
    pub fn agarrar_launchpad(&mut self) {
        if self.emergente.is_none() || self.emergente_mostrada != Some("launchpad") {
            if !self.shell.hay_emergente() {
                self.launchpad_gesto = Some(crate::gestos::Agarre::nuevo(0.0, 0.0));
            }
            return;
        }
        let mut muelle = self.muelle_emergente.get();
        let agarre = crate::gestos::Agarre::nuevo(muelle.valor(), muelle.destino());
        if !self.shell.hay_emergente() {
            self.alternar_launchpad();
        }
        self.cerrando_en = None;
        muelle.fijar(muelle.valor());
        self.muelle_emergente.set(muelle);
        self.launchpad_gesto = Some(agarre);
        self.entrada_sin_ver.set(false);
    }

    // modificado en la 0.2.0 -- el exceso en los límites no retrasa la inversión del pellizco.
    pub fn seguir_launchpad(&mut self, progreso: f32) {
        if self.launchpad_gesto.is_none() {
            self.agarrar_launchpad();
        }
        let Some(agarre) = self.launchpad_gesto.as_mut() else {
            return;
        };
        let posicion = agarre.seguir(progreso, 0.0, 1.0);
        if !self.shell.hay_emergente() {
            if posicion == 0.0 {
                return;
            }
            let agarre = *agarre;
            self.alternar_launchpad();
            self.launchpad_gesto = Some(agarre);
            self.cerrando_en = None;
        }
        self.entrada_sin_ver.set(false);
        let mut muelle = self.muelle_emergente.get();
        muelle.fijar(posicion);
        self.muelle_emergente.set(muelle);
    }

    // añadido en la 0.2.0
    pub fn soltar_launchpad(&mut self, suelta: crate::gestos::Suelta) {
        let Some(agarre) = self.launchpad_gesto.take() else {
            return;
        };
        let mut muelle = self.muelle_emergente.get();
        let abrir = agarre.destino(suelta) > 0.0;
        if !abrir {
            self.cerrar_emergente();
        }
        muelle.soltar_gesto(
            if abrir { 1.0 } else { 0.0 },
            if suelta.cancelada {
                0.0
            } else {
                suelta.velocidad
            },
        );
        self.muelle_emergente.set(muelle);
    }

    /// Hasta dónde llega el launchpad por abajo, en lógicos: el dock si se ve
    /// por delante de él, o el borde de la pantalla.
    ///
    /// Un dock que se esconde y está escondido no cuenta: se le reservaba su
    /// franja igual y los puntos de página quedaban a media pantalla sobre un
    /// hueco vacío. Si luego se le pide, sale por encima de los puntos, como
    /// en macOS.
    // 0.1.0
    // modificado en la 0.2.0 -- no reserva la franja del dock escondido
    fn alto_launchpad(&self) -> f64 {
        let dock_escondido = self.dock_barra.modo == Visibilidad::Esquivando
            && self.dock_barra.escondida
            && !self.dock_barra.pedida;
        if self.shell.dock_en_launchpad() && !dock_escondido {
            self.dock_rect().1 - SEPARACION
        } else {
            self.screen.1 as f64 / self.shell.scale() as f64
        }
    }

    /// Le pasa al launchpad dónde queda el dock, para que sepa que soltar ahí
    /// es anclar. Se llama al abrirlo y cada vez que el dock cambia de tamaño,
    /// que es lo que ocurre justo después de anclar algo.
    ///
    /// **En coordenadas del launchpad, no de la pantalla.** Su buffer no ocupa
    /// la pantalla entera —medido, 924×720 de 1280×800, centrado— aunque el
    /// velo de detrás sí; el dock cae fuera de él, y por eso la resta da
    /// números negativos o mayores que el buffer, que es lo correcto.
    // 0.1.0
    fn avisar_zona_dock(&mut self) {
        let origen = self.emergente_rect();
        let zona = self.dock_a_la_vista().then(|| {
            let (x, y, w, h) = self.dock_rect();
            let (ox, oy) = origen.map(|(ox, oy, _, _)| (ox, oy)).unwrap_or((0.0, 0.0));
            ((x - ox) as f32, (y - oy) as f32, w as f32, h as f32)
        });
        self.guard("zona del dock", |host| host.shell.launchpad_zona_dock(zona));
    }

    // 0.1.1
    pub fn alternar_emojis(&mut self, x: f32, y: f32, recientes: &[String]) {
        if self.dead {
            return;
        }
        let estaba_abierto = self.shell.emergente_nombre() == Some("emojis");
        if self.shell.hay_emergente() {
            self.cerrar_emergente();
        }
        if estaba_abierto {
            return;
        }
        self.guard("abrir selector de emojis", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::emojis(x, y, recientes));
        });
        self.sincronizar_emergente();
    }

    /// Abre o cierra el buscador. Es Meta+Espacio.
    // 0.1.0
    // modified 0.1.1 -- el buscador sabe qué aplicaciones están abiertas
    pub fn alternar_buscador(&mut self) {
        if self.dead {
            return;
        }
        // Con cualquier otra emergente abierta, esto la cierra: solo puede
        // haber una, y pedir el buscador con el menú desplegado significa que
        // quieres el buscador.
        if self.shell.hay_emergente() {
            let era_buscador = self.shell.emergente_nombre() == Some("buscador");
            self.cerrar_emergente();
            if era_buscador {
                return;
            }
        }
        // Leer los `.desktop` puede tardar unos milisegundos y va dentro del
        // guard, igual que el launchpad: un fichero raro no tumba la sesión.
        let escala = self.shell.scale();
        let pantalla = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        self.guard("abrir buscador", |host| {
            // Las abiertas, del dock: es quien ya empareja ventanas con su
            // `.desktop`, y así el buscador y el punto bajo el icono no pueden
            // discrepar.
            let abiertas = host
                .shell
                .dock()
                .items()
                .iter()
                .filter(|i| i.abierta())
                .map(|i| (i.exec().to_string(), i.app_id().to_string()))
                .collect();
            host.shell
                .abrir(bookos_shell::Emergente::buscador_en(pantalla, abiertas));
        });
        self.sincronizar_emergente();
    }

    /// Abre o cierra la vista general de escritorios.
    // 0.1.0
    pub fn alternar_vista_escritorios(&mut self, activo: usize, nombres: Vec<String>) {
        if self.dead {
            return;
        }
        if self.shell.emergente_nombre() == Some("escritorios") {
            self.cerrar_emergente();
            return;
        }
        let escala = self.shell.scale();
        let pantalla = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        self.guard("abrir vista de escritorios", |host| {
            host.shell.abrir(bookos_shell::Emergente::escritorios(
                pantalla, activo, nombres,
            ));
        });
        self.sincronizar_emergente();
    }

    /// Cuatro dedos en vertical con la vista de escritorios: subir la abre y,
    /// abierta, bajar la cierra, siguiendo a la mano. Comparte el muelle de
    /// apertura con el launchpad, así que la tarjeta y las ventanas (que leen
    /// [`Self::avance_emergente`]) van a la vez.
    // añadido en la 0.2.0
    pub fn seguir_vista(&mut self, progreso: f32, activo: usize, nombres: Vec<String>) {
        if self.vista_gesto.is_none() {
            let muelle = self.muelle_emergente.get();
            self.vista_gesto = Some(if self.vista_escritorios_abierta() {
                crate::gestos::Agarre::nuevo(muelle.valor().clamp(0.0, 1.0), muelle.destino())
            } else {
                crate::gestos::Agarre::nuevo(0.0, 0.0)
            });
        }
        let Some(agarre) = self.vista_gesto.as_mut() else {
            return;
        };
        let posicion = agarre.seguir(progreso, 0.0, 1.0);
        let agarre = *agarre;
        if !self.vista_escritorios_abierta() {
            if posicion == 0.0 {
                return;
            }
            self.alternar_vista_escritorios(activo, nombres);
            self.vista_gesto = Some(agarre);
            self.cerrando_en = None;
        }
        self.entrada_sin_ver.set(false);
        let mut muelle = self.muelle_emergente.get();
        muelle.fijar(posicion);
        self.muelle_emergente.set(muelle);
    }

    // añadido en la 0.2.0
    pub fn soltar_vista(&mut self, suelta: crate::gestos::Suelta) {
        let Some(agarre) = self.vista_gesto.take() else {
            return;
        };
        let mut muelle = self.muelle_emergente.get();
        let abrir = agarre.destino(suelta) > 0.0;
        if !abrir {
            self.cerrar_emergente();
        }
        muelle.soltar_gesto(
            if abrir { 1.0 } else { 0.0 },
            if suelta.cancelada {
                0.0
            } else {
                suelta.velocidad
            },
        );
        self.muelle_emergente.set(muelle);
    }

    /// Cuánto de abierta está la tarjeta en este instante, de 0 a 1.
    // añadido en la 0.2.0
    pub fn avance_emergente(&self) -> f32 {
        self.muelle_emergente.get().valor().clamp(0.0, 1.0)
    }

    // 0.1.0
    pub fn actualizar_vista_escritorios(&mut self, activo: usize, nombres: Vec<String>) {
        if self.dead {
            return;
        }
        self.guard("actualizar vista de escritorios", |host| {
            host.shell.actualizar_vista_escritorios(activo, nombres);
        });
        self.sincronizar_emergente();
    }

    // 0.1.0
    pub fn vista_escritorios_abierta(&self) -> bool {
        !self.dead && self.shell.emergente_nombre() == Some("escritorios")
    }

    pub fn vista_escritorios_visible(&self) -> bool {
        !self.dead && self.emergente_mostrada == Some("escritorios") && self.emergente.is_some()
    }

    /// Dónde acaba por abajo la franja de la vista de escritorios, en lógicos
    /// globales. Debajo se reparten las ventanas del escritorio actual.
    // 0.1.0
    pub fn vista_escritorios_fondo(&self) -> Option<i32> {
        let surface = self
            .emergente
            .as_ref()
            .filter(|_| self.vista_escritorios_abierta())?;
        let escala = self.shell.scale() as f64;
        Some(((surface.location.y + surface.src.size.h) / escala).round() as i32)
    }

    /// Huecos globales lógicos en los que la escena monta cada escritorio.
    // 0.1.0
    pub fn escritorios_miniaturas(&self) -> Vec<Rectangle<i32, Logical>> {
        if !self.vista_escritorios_visible() {
            return Vec::new();
        }
        let desplazamiento = self.animacion().2.round() as i32;
        self.miniaturas_vista
            .iter()
            .map(|r| Rectangle::new((r.loc.x, r.loc.y + desplazamiento).into(), r.size))
            .collect()
    }

    fn miniaturas_vista_sin_animar(&self) -> Vec<Rectangle<i32, Logical>> {
        let Some(surface) = self.emergente.as_ref() else {
            return Vec::new();
        };
        let escala = self.shell.scale() as f64;
        let origen_x = surface.location.x / escala;
        let origen_y = surface.location.y / escala;
        self.shell
            .escritorios_miniaturas()
            .into_iter()
            .map(|r| {
                Rectangle::new(
                    (
                        (origen_x + r.x as f64).round() as i32,
                        (origen_y + r.y as f64).round() as i32,
                    )
                        .into(),
                    (
                        r.width.round().max(1.0) as i32,
                        r.height.round().max(1.0) as i32,
                    )
                        .into(),
                )
            })
            .collect()
    }

    /// Le dice al dock qué aplicaciones tienen ventana abierta.
    ///
    /// Los `app_id` los reúne el compositor, que es quien conoce el `Space`. El
    /// shell no lo mira por su cuenta: la frontera con Smithay está puesta a
    /// propósito para que cambiar de toolkit no obligue a tocar el compositor.
    /// Le pasa al panel en qué escritorio está la sesión.
    ///
    /// El panel puede cambiar de ancho —el indicador aparece en cuanto se sabe
    /// cuántos escritorios hay—, así que se repinta por el mismo camino que
    /// cualquier otro cambio de sus widgets.
    // 0.1.0
    pub fn escritorios(&mut self, activo: usize, cuantos: usize) {
        if self.dead {
            return;
        }
        let repintar = self
            .guard("escritorios", |host| {
                host.shell.escritorios(activo, cuantos)
            })
            .unwrap_or(false);
        if repintar {
            // La superficie no se rehace: el panel ocupa el ancho de la
            // pantalla y eso no cambia porque aparezca un widget. Lo que cambia
            // es su contenido y el reparto de las zonas de clic, que sale del
            // ancho de cada widget en el siguiente `zonas_panel`.
            self.guard("pintar panel", |host| {
                let buffer = &mut host.panel.buffer;
                paint(buffer, |buf| host.shell.draw_panel(buf));
            });
        }
    }

    // 0.1.0
    pub fn ventanas(&mut self, app_ids: &[String]) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("ventanas", |host| host.shell.dock_ventanas(app_ids))
            .unwrap_or(false);
        if repintar {
            // Recolocar antes de pintar: el dock pudo cambiar de ancho al
            // aparecer el icono de una aplicación abierta sin lanzador.
            self.dock = Surface::new(
                self.shell.dock_buffer_size(),
                self.shell.dock_logical_size(),
            );
            self.place_dock();
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }
        repintar
    }

    /// Desplazamiento de rueda o touchpad sobre el shell.
    ///
    /// Devuelve `true` si lo ha consumido, en cuyo caso **no** debe llegar al
    /// cliente: desplazarse por el launchpad no puede además hacer scroll en la
    /// ventana que hay debajo.
    // 0.1.0
    pub fn desplazar(&mut self, dx: f64, dy: f64) -> bool {
        if self.dead || !self.shell.hay_emergente() {
            return false;
        }
        let repintar = self
            .guard("desplazar", |host| {
                host.shell.emergente_desplazar(dx as f32, dy as f32)
            })
            .unwrap_or(false);
        if repintar {
            let delta = if dx.abs() >= dy.abs() { dx } else { dy };
            let hacia = if delta >= 0.0 { 1.0 } else { -1.0 };
            if self.shell.emergente_nombre() == Some("launchpad")
                && let Some(anterior) = self.emergente.take()
            {
                let mut transicion =
                    bookos_shell::tema::Transicion::nueva(0.0, bookos_shell::tema::M_SUAVE);
                transicion.ir_a(1.0);
                self.pagina_anterior = Some((anterior, transicion, hacia));
            }
            self.sincronizar_emergente();
            // La superficie nueva es otra página de la misma emergente: no
            // debe sumar el zoom de apertura al deslizamiento lateral.
        }
        // Consumido aunque no haya cambiado de página: el gesto es del
        // launchpad de principio a fin.
        true
    }

    /// Botón derecho dentro del Launchpad: entra o sale de edición.
    // 0.1.0
    pub fn editar_launchpad(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let cambio = self
            .guard("editar launchpad", |host| host.shell.launchpad_editar())
            .unwrap_or(false);
        if cambio {
            self.sincronizar_emergente();
        }
        cambio
    }

    /// Qué hacer al pulsar en un punto lógico. `None` si ahí no hay nada.
    // 0.1.0
    // modified 0.1.1 -- el dock arrastra iconos para reordenar o sacarlos, también sin launchpad
    pub fn pulsar(&mut self, x: f64, y: f64) -> Option<Accion> {
        if self.dead {
            return None;
        }

        // El aviso de una notificación gana a todo lo que tenga debajo: está
        // encima de las ventanas, y un clic que lo atravesara escribiría en una
        // aplicación que el usuario ni siquiera está mirando.
        if let Some((id, pulsado)) = self.toast_accion(x, y) {
            use bookos_shell::toast::Pulsado;
            return Some(match pulsado {
                Pulsado::Boton(clave) => Accion::NotificacionAccion { id, clave },
                // La ✕ borra: la quita también de la lista y avisa a la app.
                Pulsado::Cerrar => Accion::CerrarNotificacion(id),
            });
        }
        // Pulsar el cuerpo lo despide y, si la app ofrece `default`, la
        // ejecuta: es lo que abre la conversación de un aviso de mensajería.
        let defecto = self
            .toast_bajo(x, y)
            .and_then(|(id, _)| Some((id, self.shell.toast_accion_por_defecto(id)?)));
        if self.toast_pulsado(x, y) {
            return defecto.map(|(id, clave)| Accion::NotificacionAccion { id, clave });
        }

        if self.asistente_minimizado()
            && Self::dentro(self.actividad_rect(), x, y).is_some()
            && let Some((bx, by)) = self.burbuja_asistente
        {
            self.arrastre_burbuja = Some(ArrastreBurbuja {
                agarre: (x - bx, y - by),
                inicio: (x, y),
                movido: false,
            });
            return None;
        }
        if let Some((ax, ay)) = Self::dentro(self.actividad_rect(), x, y) {
            let margen = bookos_shell::actividad::MARGEN_SOMBRA;
            // `dentro` mide desde la zona visible; la isla, desde su buffer.
            let x0 = self.shell.actividad_zona().map_or(0.0, |(x0, _)| x0);
            let accion = self
                .guard("pulsar actividad", |host| {
                    host.shell.actividad_pulsar(ax + margen + x0, ay + margen)
                })
                .flatten()
                .map(Accion::Actividad);
            self.pintar_actividad();
            self.reclamar_bajo_el_puntero();
            return accion;
        }

        // El panel gana al «pulsar fuera cierra» de la emergente. Con una
        // tarjeta abierta, un clic en otro widget caía fuera de ella y se
        // gastaba solo en cerrarla: había que pulsar dos veces para cambiar de
        // tarjeta, que es justo lo que hacía que abrir los widgets «costara».
        // Una barra de menús pasa de un menú al siguiente con un clic, y
        // `panel_pulsado` ya sabe alternar cuando se repite el mismo widget.
        // Salvo cuando lo que hay abierto tapa la pantalla entera: el
        // launchpad se dibuja **encima** del panel, y ahí arriba lo que hay es
        // launchpad, no widgets.
        if y < self.shell.panel_height() as f64 && !self.shell.emergente_tapa_la_pantalla() {
            let objetivo = self.shell.objetivo_emergente_panel(x as f32);
            let abierta = self.shell.emergente_nombre();
            if let (Some(abierta), Some((widget, nombre_objetivo))) = (abierta, objetivo) {
                if abierta == nombre_objetivo {
                    // Segundo clic en el mismo widget: cerrar con la animación
                    // del host, no destruyendo el buffer de golpe en el shell.
                    self.cerrar_emergente();
                    return None;
                }
                self.cambiar_a_widget(widget);
                return None;
            }
            let accion = self
                .guard("pulsar panel", |host| {
                    host.shell.panel_pulsado(x as f32, y as f32)
                })
                .flatten();
            self.sincronizar_emergente();
            return accion;
        }

        // El launchpad ocupa toda la pantalla, pero la franja visible del
        // dock sigue siendo suya. Los menús conservan su «pulsar fuera cierra».
        if self.dock_a_la_vista()
            && self.a_la_vista(Barra::Dock)
            && (!self.shell.hay_emergente() || self.shell.emergente_nombre() == Some("launchpad"))
            && let Some((ex, ey)) = Self::dentro(Some(self.dock_rect()), x, y)
        {
            let (app_id, exec, icono) = self.shell.dock_item_en(ex, ey)?;
            let clic = self.shell.dock_pulsar(ex, ey)?;
            let i = self
                .shell
                .dock()
                .item_en(ex, ey)
                .expect("el icono se encontró en el mismo punto");
            let origen_x = self.shell.dock().centro_de(i);
            let lado = f64::from(self.shell.dock().tamano_actual());
            let (dx, dy, _, dh) = self.dock_rect();
            self.puntero_en = (x, y);
            self.arrastre_dock = Some(ArrastreDock {
                app_id,
                exec,
                icono,
                clic,
                inicio: (x, y),
                desfase: (
                    x - dx - f64::from(origen_x) + lado / 2.0,
                    y - dy - (dh - lado) / 2.0,
                ),
                origen_x,
                estado: EstadoArrastreDock::Pulsado,
            });
            self.reclamar_bajo_el_puntero();
            return None;
        }

        if self.shell.hay_emergente() {
            match Self::dentro(self.emergente_rect(), x, y) {
                Some((ex, ey)) => {
                    let accion = self
                        .guard("pulsar emergente", |host| {
                            host.shell.emergente_pulsar(ex, ey)
                        })
                        .flatten();
                    self.sincronizar_emergente();
                    return accion;
                }
                // Fuera de la emergente: se cierra y el clic muere ahí. Que
                // además activase lo que hubiera debajo sería una sorpresa
                // desagradable — cerrar un menú no es pulsar el escritorio.
                None => {
                    self.cerrar_emergente();
                    return None;
                }
            }
        }

        None
    }

    /// Enseña el aviso de volumen, brillo y demás, centrado sobre el dock.
    // 0.1.0
    pub fn mostrar_osd(&mut self, icono: &str, nivel: Option<u8>, texto: Option<String>) {
        self.guard("osd", |host| host.shell.mostrar_osd(icono, nivel, texto));
        self.pintar_osd();
        reproducir_sonido_osd(icono);
    }

    /// Aplica el tema y el acento elegidos en Apariencia, y repinta el shell.
    ///
    /// Devuelve `false` si el shell está caído: entonces no hay nada que
    /// guardar tampoco, porque el usuario no ha podido pulsar nada.
    // 0.1.0
    // modified 0.1.1 -- repinta el teclado en pantalla con el acento nuevo
    pub fn aplicar_apariencia(
        &mut self,
        tema: bookos_shell::tema::Tema,
        acento: bookos_shell::tema::Acento,
    ) -> bool {
        if self.dead {
            return false;
        }
        // Solo el cambio de claro a oscuro se funde: un acento nuevo cambia el
        // color de cuatro detalles y fundirlo no aporta nada. Si ya había uno
        // esperando se queda el primero, que es lo que hay en pantalla.
        let antes = (
            bookos_shell::tema::actual(),
            bookos_shell::tema::acento_actual(),
        );
        if antes.0 != tema && !bookos_shell::tema::efectos_reducidos() {
            self.tema_anterior.get_or_insert(antes);
        }
        let hecho = self
            .guard("apariencia", |host| {
                host.shell.aplicar_apariencia(tema, acento);
                // El teclado se tiñe del acento: lo repinta `teclado_tick`.
                host.teclado_invalidar();
                let buffer = &mut host.panel.buffer;
                paint(buffer, |buf| host.shell.draw_panel(buf));
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
                // Y los iconos del escritorio, que llevan el acento en la
                // pastilla del nombre y en el realce de lo seleccionado.
                for i in 0..host.escritorio.len() {
                    host.pintar_celda(i);
                }
            })
            .is_some();
        // La emergente va aparte: su superficie se rehace, no se repinta en un
        // buffer que ya está puesto.
        if hecho {
            self.sincronizar_emergente();
        }
        hecho
    }

    /// El logo del panel con el color del canal de actualizaciones.
    // 0.1.0
    pub fn aplicar_canal(&mut self, canal: bookos_shell::marca::Canal) {
        if self.dead {
            return;
        }
        self.guard("canal", |host| {
            host.shell.aplicar_canal(canal);
            let buffer = &mut host.panel.buffer;
            paint(buffer, |buf| host.shell.draw_panel(buf));
        });
    }

    /// Recarga la composición del bloqueo guardada por BookOS Settings.
    // 0.1.0
    pub fn aplicar_bloqueo_config(&mut self, config: bookos_shell::ConfigBloqueo) -> bool {
        if self.dead {
            return false;
        }
        let visible = self
            .guard("ajustes del bloqueo", |host| {
                host.shell.aplicar_bloqueo_config(config)
            })
            .unwrap_or(false);
        if visible {
            self.repintar_bloqueo(&bookos_shell::bloqueo::Capa::TODAS);
        }
        true
    }

    // 0.1.0
    pub fn aplicar_actividades_config(&mut self, config: bookos_shell::ConfigActividades) -> bool {
        if self.dead {
            return false;
        }
        self.guard("ajustes de actividades", |host| {
            host.shell.aplicar_actividades_config(config)
        });
        self.pintar_actividad();
        true
    }

    // 0.1.0
    pub fn aplicar_efectos_config(&mut self, efectos: bookos_shell::Efectos) -> bool {
        if self.dead {
            return false;
        }
        self.guard("ajustes de efectos", |host| {
            host.shell.aplicar_efectos_config(efectos)
        })
        .unwrap_or(false)
    }

    /// Repinta la emergente si su contenido se está moviendo.
    ///
    /// Lo llama el compositor antes de componer cada frame: la transición de
    /// página del launchpad ocurre **dentro** del buffer, así que nadie más lo
    /// marcaría sucio.
    // 0.1.0
    // modificado en la 0.2.0 -- recoge los arrastres del puntero, como mucho uno cada 6 ms
    pub fn animar_emergente(&mut self) {
        // modificado en la 0.2.0 -- avanzar no rasteriza: el gesto y la apertura usan la textura existente.
        if self.launchpad_gesto.is_none()
            && self.vista_gesto.is_none()
            && !self.entrada_sin_ver.get()
        {
            let mut muelle = self.muelle_emergente.get();
            muelle.avanzar(std::time::Instant::now());
            self.muelle_emergente.set(muelle);
        }
        self.recoger_cerrada();
        // El bucle pasa por aquí tras cada lote de entrada, también con un
        // fotograma en vuelo: sin este tope se pintaría más de una vez por
        // refresco. 6 ms y no 8,3 (120 Hz), para que el vblank no lo encuentre
        // a punto de cumplirse y se salte un fotograma.
        let arrastre = self.emergente_por_pintar
            && self.arrastre_pintado_en.elapsed() >= std::time::Duration::from_millis(6);
        if self.dead || !(arrastre || self.shell.emergente_animando()) {
            return;
        }
        if arrastre {
            self.emergente_por_pintar = false;
            self.arrastre_pintado_en = std::time::Instant::now();
        }
        self.sincronizar_emergente();
    }

    // --- Actividades vivas -------------------------------------------------

    // 0.1.0
    pub fn publicar_actividad(&mut self, estado: bookos_shell::actividad::Estado) {
        self.guard("publicar actividad", |host| {
            host.shell.publicar_actividad(estado)
        });
        self.actividad_visible = true;
        self.pintar_actividad();
    }

    /// La aplicación con el foco cambió: su actividad sale de la isla o vuelve.
    // 0.1.0
    pub fn poner_app_enfocada(&mut self, app_id: Option<String>) {
        self.guard("app enfocada", |host| host.shell.poner_app_enfocada(app_id));
        self.pintar_actividad();
    }

    // 0.1.0
    pub fn cerrar_actividad(&mut self, app_id: &str) -> bool {
        let cerrada = self.guard("cerrar actividad", |host| {
            host.shell.cerrar_actividad(app_id)
        }) == Some(true);
        if cerrada {
            self.pintar_actividad();
            self.reclamar_bajo_el_puntero();
        }
        cerrada
    }

    // 0.1.0
    pub fn abrir_actividad_previsualizacion(&mut self, app_id: &str) -> bool {
        let abierta = self.guard("abrir actividad de previsualización", |host| {
            host.shell.abrir_actividad_previsualizacion(app_id)
        }) == Some(true);
        if abierta {
            self.pintar_actividad();
        }
        abierta
    }

    // 0.1.0
    pub fn alternar_actividad(&mut self) -> bool {
        let cambiada =
            self.guard("alternar actividad", |host| host.shell.alternar_actividad()) == Some(true);
        if cambiada {
            self.pintar_actividad();
            self.reclamar_bajo_el_puntero();
        }
        cambiada
    }

    // 0.1.0
    pub fn actividad_app_id(&self) -> Option<&str> {
        self.shell.actividad_app_id()
    }

    // 0.1.0
    pub fn actividad_clase(&self) -> Option<bookos_shell::actividad::Clase> {
        self.shell.actividad_clase()
    }

    // añadido en la 0.2.0
    pub fn actividad_nivel(&self) -> f32 {
        self.shell.actividad_nivel()
    }

    // 0.1.0
    pub fn actividad_visible(&mut self, visible: bool) -> bool {
        if self.actividad_visible == visible {
            return false;
        }
        self.actividad_visible = visible;
        true
    }

    // 0.1.0
    fn pintar_actividad(&mut self) {
        let Some((w, h)) = self.shell.actividad_buffer_size() else {
            self.actividad = None;
            return;
        };
        let Some((lw, lh)) = self.shell.actividad_logical_size() else {
            return;
        };
        let rehacer = self
            .actividad
            .as_ref()
            .is_none_or(|s| s.src.size.w != w as f64 || s.src.size.h != h as f64);
        if rehacer {
            self.actividad = Some(Surface::new((w, h), (lw as i32, lh as i32)));
        }
        let escala = self.shell.scale() as f64;
        // La isla se maqueta para el panel de 32: en uno de otro alto va
        // centrada en él, sin crecer con su zoom.
        let arriba = bookos_shell::actividad::ARRIBA as f64
            + (self.shell.panel_height() - bookos_shell::PANEL_HEIGHT as i32) as f64 / 2.0;
        let burbuja = self.asistente_minimizado().then(|| self.colocar_burbuja());
        if let Some(s) = self.actividad.as_mut() {
            let sombra = bookos_shell::actividad::MARGEN_SOMBRA as f64;
            s.location = match burbuja {
                Some((x, y)) => ((x - sombra) * escala, (y - sombra) * escala),
                None => (
                    (self.screen.0 as f64 - w as f64) / 2.0,
                    (arriba - sombra) * escala,
                ),
            }
            .into();
            let shell = &mut self.shell;
            paint(&mut s.buffer, |buf| shell.draw_actividad(buf));
        }
    }

    /// Zenith minimizado: la isla deja la barra y es una burbuja que se mueve.
    // añadido en la 0.2.0
    fn asistente_minimizado(&self) -> bool {
        self.shell.actividad_clase()
            == Some(bookos_shell::actividad::Clase::Asistente(
                bookos_shell::actividad::Fase::Minimizado,
            ))
    }

    /// Zenith no se aparta con el panel: si la barra está oculta, hablarle sin
    /// ver que escucha es no saber si el micro está abierto. Lo demás de la isla
    /// sí se va con ella, porque tapaba la cabecera de las ventanas.
    // añadido en la 0.2.0
    fn isla_sin_panel(&self) -> bool {
        matches!(
            self.shell.actividad_clase(),
            Some(bookos_shell::actividad::Clase::Asistente(_))
        )
    }

    /// Dónde va la burbuja, dentro de la pantalla y sin pisar panel ni dock.
    // añadido en la 0.2.0
    fn colocar_burbuja(&mut self) -> (f64, f64) {
        let escala = self.shell.scale() as f64;
        let lado = bookos_shell::actividad::ASIS_MINI as f64;
        let (ancho, alto) = (self.screen.0 as f64 / escala, self.screen.1 as f64 / escala);
        let (x, y) = self
            .burbuja_asistente
            .unwrap_or((ancho - MARGEN_BURBUJA - lado, (alto - lado) / 2.0));
        let arriba = self.shell.panel_height() as f64 + MARGEN_BURBUJA;
        let abajo = alto - self.shell.dock_logical_size().1 as f64 - MARGEN_BURBUJA - lado;
        let colocada = (
            x.clamp(0.0, (ancho - lado).max(0.0)),
            y.clamp(arriba, abajo.max(arriba)),
        );
        self.burbuja_asistente = Some(colocada);
        colocada
    }

    /// Mueve la burbuja cogida sin repintarla: solo cambia dónde se compone.
    // añadido en la 0.2.0
    fn arrastrar_burbuja(&mut self, x: f64, y: f64) {
        let Some(arrastre) = self.arrastre_burbuja.as_mut() else {
            return;
        };
        // Cuatro puntos de holgura: la mano tiembla al hacer clic y eso no
        // es querer moverla.
        arrastre.movido |= (x - arrastre.inicio.0).hypot(y - arrastre.inicio.1) > 4.0;
        let nueva = (x - arrastre.agarre.0, y - arrastre.agarre.1);
        self.burbuja_asistente = Some(nueva);
        let escala = self.shell.scale() as f64;
        let sombra = bookos_shell::actividad::MARGEN_SOMBRA as f64;
        if let Some(s) = self.actividad.as_mut() {
            s.location = ((nueva.0 - sombra) * escala, (nueva.1 - sombra) * escala).into();
        }
    }

    /// Al soltar se pega al lado más cercano, como la burbuja de Gemini; un
    /// clic sin arrastre le pide a Zenith que vuelva a abrirse.
    // añadido en la 0.2.0
    fn soltar_burbuja(&mut self, arrastre: ArrastreBurbuja) -> Option<Accion> {
        if !arrastre.movido {
            return Some(Accion::Actividad(bookos_shell::actividad::Accion {
                app_id: self.actividad_app_id()?.to_string(),
                nombre: "expand".into(),
                valor: String::new(),
            }));
        }
        let escala = self.shell.scale() as f64;
        let lado = bookos_shell::actividad::ASIS_MINI as f64;
        let ancho = self.screen.0 as f64 / escala;
        if let Some((x, y)) = self.burbuja_asistente {
            let x = if x + lado / 2.0 < ancho / 2.0 {
                MARGEN_BURBUJA
            } else {
                ancho - MARGEN_BURBUJA - lado
            };
            self.burbuja_asistente = Some((x, y));
        }
        self.pintar_actividad();
        None
    }

    /// Un clic fuera de Zenith lo quita de en medio. Como al recoger, el clic
    /// sigue su camino: quien pulsa fuera iba a otra cosa.
    // añadido en la 0.2.0
    pub fn despedir_asistente_fuera(&mut self, x: f64, y: f64) -> Option<Accion> {
        if self.dead
            || !self.actividad_visible
            || Self::dentro(self.actividad_rect(), x, y).is_some()
        {
            return None;
        }
        self.guard("despedir asistente", |host| {
            host.shell.despedir_asistente(false)
        })
        .flatten()
        .map(Accion::Actividad)
    }

    /// Un clic fuera de la tarjeta abierta la recoge. No se come el clic: el
    /// usuario pulsaba otra cosa y tiene que llegarle.
    // 0.1.0
    pub fn recoger_actividad_fuera(&mut self, x: f64, y: f64) -> bool {
        if self.dead || Self::dentro(self.actividad_rect(), x, y).is_some() {
            return false;
        }
        let recogida =
            self.guard("recoger actividad", |host| host.shell.recoger_actividad()) == Some(true);
        if recogida {
            self.pintar_actividad();
            self.reclamar_bajo_el_puntero();
        }
        recogida
    }

    // 0.1.0
    pub fn animar_actividad(&mut self) {
        if !self.dead
            && self.guard("terminar salida de la isla", |host| {
                host.shell.terminar_salida_actividad()
            }) == Some(true)
        {
            self.pintar_actividad();
            self.reclamar_bajo_el_puntero();
            return;
        }
        if self.dead || !self.actividad_visible || !self.shell.actividad_needs_paint() {
            return;
        }
        self.pintar_actividad();
    }

    /// ¿Cae el punto sobre la isla? La vista de escritorios lo pregunta antes
    /// de coger la ventana de una miniatura: la tarjeta desplegada se dibuja
    /// delante de ellas y el clic es suyo.
    // 0.1.0
    pub fn sobre_actividad(&self, x: f64, y: f64) -> bool {
        !self.dead && Self::dentro(self.actividad_rect(), x, y).is_some()
    }

    // 0.1.0
    fn actividad_rect(&self) -> Option<(f64, f64, f64, f64)> {
        // Con el panel apartado la isla se va con él (ver `Papel::Actividad`),
        // y su zona tampoco puede quedarse: se comía los clics a la ventana
        // de debajo justo en el centro de su cabecera.
        if !self.actividad_visible || (!self.a_la_vista(Barra::Panel) && !self.isla_sin_panel()) {
            return None;
        }
        let s = self.actividad.as_ref()?;
        let escala = self.shell.scale() as f64;
        let sombra = bookos_shell::actividad::MARGEN_SOMBRA as f64;
        // Solo lo que se ve: con burbujas, el buffer reserva el mismo ancho a
        // la izquierda para dejar la píldora centrada, y ese hueco es panel.
        let (x0, ancho) = self.shell.actividad_zona()?;
        Some((
            s.location.x / escala + sombra + x0 as f64,
            s.location.y / escala + sombra,
            ancho as f64,
            s.logical.h as f64 - sombra * 2.0,
        ))
    }

    // --- Notificaciones ----------------------------------------------------

    /// Guarda una notificación y repinta lo que la enseña.
    // 0.1.0
    pub fn notificar(
        &mut self,
        notificacion: bookos_shell::notificaciones::Notificacion,
        caducidad: i32,
    ) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("notificar", |host| {
                host.shell.notificar(notificacion, caducidad)
            })
            .unwrap_or(false);
        self.pintar_toasts();
        self.tras_notificar(repintar)
    }

    // 0.1.1
    pub fn archivar_notificacion(
        &mut self,
        notificacion: bookos_shell::notificaciones::Notificacion,
    ) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("archivar notificación", |host| {
                host.shell.archivar_notificacion(notificacion)
            })
            .unwrap_or(false);
        self.tras_notificar(repintar)
    }

    /// Crea las superficies de los avisos que no la tienen, repinta las que
    /// lo piden y suelta las de los que ya se fueron.
    ///
    /// Todas se colocan en la **cabeza** de la pila, arriba a la derecha: el
    /// sitio de cada una dentro de la pila cambia con cada fotograma mientras
    /// otra entra o sale, y se suma al componer (ver
    /// [`Self::toasts_desplazamientos`]).
    // 0.1.0
    fn pintar_toasts(&mut self) {
        let ids = self.shell.toasts_ids();
        self.toasts.retain(|(id, _)| ids.contains(id));
        let escala = self.shell.scale() as f64;
        // Pegado a la esquina, bajo el panel. Los márgenes se cuentan desde la
        // **tarjeta**, no desde el buffer, que lleva su hueco para la sombra.
        let margen = (bookos_shell::toast::MARGEN_LATERAL - bookos_shell::toast::MARGEN_SOMBRA)
            as f64
            * escala;
        let alto_panel = self.shell.panel_height() as f64 * escala;
        let arriba = (bookos_shell::toast::MARGEN_SUPERIOR - bookos_shell::toast::MARGEN_SOMBRA)
            as f64
            * escala;
        for id in ids {
            let (Some((w, h)), Some((lw, lh))) = (
                self.shell.toast_buffer_size(id),
                self.shell.toast_logical_size(id),
            ) else {
                continue;
            };
            let logico = Size::<i32, Logical>::from((lw as i32, lh as i32));
            let hay = self
                .toasts
                .iter()
                .position(|(t, s)| *t == id && s.logical == logico);
            let indice = match hay {
                Some(_) if !self.shell.toast_needs_paint(id) => continue,
                Some(i) => i,
                None => {
                    self.toasts.retain(|(t, _)| *t != id);
                    let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
                    surface.location = (
                        self.screen.0 as f64 - w as f64 - margen,
                        alto_panel + arriba,
                    )
                        .into();
                    self.toasts.push((id, surface));
                    self.toasts.len() - 1
                }
            };
            let shell = &mut self.shell;
            paint(&mut self.toasts[indice].1.buffer, |buf| {
                shell.draw_toast(id, buf)
            });
        }
    }

    /// Cuánto baja cada aviso respecto al de delante, en lógicos, en el orden
    /// en que se componen: el de delante primero, que en la lista de Smithay es
    /// el que queda encima. Todos nacen en el mismo sitio y el nivel los
    /// desplaza.
    // 0.1.0
    fn toasts_desplazamientos(&self) -> Vec<(u32, f32)> {
        self.shell
            .toasts_para_componer()
            .into_iter()
            .filter_map(|id| Some((id, self.shell.toast_estado(id)?.2)))
            .collect()
    }

    // --- Las barras de título de las ventanas -------------------------------

    /// El elemento de la barra de la ventana `id`, ya pintada si hacía falta.
    ///
    /// `origen` es la esquina superior izquierda de la barra en píxeles
    /// **físicos**, y `ancho` su anchura en lógicos —la de la ventana—. Devuelve
    /// `None` si el shell está muerto o la textura no se pudo subir: una sesión
    /// sin barras es un fallo que se mira, quedarse sin ventanas no.
    // 0.1.0
    pub fn barra_ventana<R>(
        &mut self,
        renderer: &mut R,
        id: u64,
        ancho: i32,
        estado: bookos_shell::decoracion::Estado,
        origen: Point<f64, Physical>,
        alfa: f32,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead || ancho <= 0 {
            return None;
        }
        let repintar = self.guard("preparar barra de título", |host| {
            host.shell.barra_preparar(id, ancho as f32, estado)
        })?;
        let (w, h) = self.shell.barra_buffer_size(id)?;
        let (lw, lh) = self.shell.barra_logical_size(id)?;
        // El buffer se rehace solo cuando cambia de tamaño: redimensionar una
        // ventana es lo único que lo obliga, y mientras el ancho no cambie el
        // repintado va sobre el que ya está subido.
        let surface = match self.barras.get(&id) {
            Some(s) if s.src.size.w == w as f64 && s.src.size.h == h as f64 => {
                self.barras.get_mut(&id)?
            }
            _ => self
                .barras
                .entry(id)
                .insert_entry(Surface::new((w, h), (lw as i32, lh as i32)))
                .into_mut(),
        };
        surface.location = origen;
        if repintar {
            let shell = &mut self.shell;
            paint(&mut surface.buffer, |buf| shell.draw_barra(id, buf));
        }
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            surface.location,
            &surface.buffer,
            Some(alfa),
            Some(surface.src),
            Some(surface.logical),
            Kind::Unspecified,
        )
        .inspect_err(|_| tracing::warn!("no se pudo subir la barra de título"))
        .ok()
    }

    /// Tira las barras de las ventanas que ya no existen.
    // 0.1.0
    // modified 0.1.1 -- purga las barras aunque coincida el número
    pub fn barras_retener(&mut self, vivas: &[u64]) {
        self.barras.retain(|id, _| vivas.contains(id));
        self.guard("purgar barras de título", |host| {
            host.shell.barras_retener(vivas)
        });
    }

    /// ¿Sigue algún aviso a la vista? Devuelve además si **acaba** de
    /// retirarse alguno, porque entonces hace falta un frame más: el que lo
    /// quita de la pantalla.
    // 0.1.0
    pub fn toasts_vivos(&mut self) -> (bool, bool) {
        let vivo = self.guard("toast vivo", |host| host.shell.toasts_vivos()) == Some(true);
        let antes = self.toasts.len();
        let ids = self.shell.toasts_ids();
        self.toasts.retain(|(id, _)| ids.contains(id));
        (vivo, self.toasts.len() < antes)
    }

    // 0.1.0
    pub fn toasts_quedan(&self) -> Vec<std::time::Duration> {
        self.shell.toasts_quedan()
    }

    /// Repinta los avisos que lo piden.
    // 0.1.0
    pub fn animar_toast(&mut self) {
        if self.dead {
            return;
        }
        self.guard("animar toast", |host| host.pintar_toasts());
    }

    /// El aviso bajo el punto y el rectángulo de su **tarjeta** en lógicos: el
    /// buffer lleva el margen de la sombra, que no se pulsa. Se mira en el
    /// sitio donde está ahora dentro de la pila.
    ///
    /// En lógicos, como `actividad_rect` y como el punto que llega de la
    /// entrada. Estaba en físicos: a escala 1 daba igual, pero a 1,75 o 2 el
    /// rectángulo caía fuera de la pantalla y ningún clic llegaba nunca a un
    /// aviso —ni sus botones ni el clic para despedirlo—.
    // 0.1.0
    // modified 0.1.1 -- cuenta el deslizamiento lateral del aviso al pulsarlo
    fn toast_bajo(&self, x: f64, y: f64) -> Option<(u32, (f64, f64, f64, f64))> {
        let escala = self.shell.scale() as f64;
        let sombra = bookos_shell::toast::MARGEN_SOMBRA as f64;
        // Solo el de delante: los de atrás son franjas sin contenido, y un clic
        // sobre ellas no es pulsar un aviso.
        let id = self.shell.toast_de_delante()?;
        let (_, s) = self.toasts.iter().find(|(t, _)| *t == id)?;
        let dx = self.shell.toast_desplazamiento_horizontal(id)? as f64;
        let rect = (
            s.location.x / escala + sombra + dx,
            s.location.y / escala + sombra,
            s.logical.w as f64 - sombra * 2.0,
            s.logical.h as f64 - sombra * 2.0,
        );
        Self::dentro(Some(rect), x, y).map(|_| (id, rect))
    }

    /// ¿Se ha pulsado un aviso? Entonces se va, y el clic no llega a la ventana
    /// de debajo: pulsar un aviso es despedirlo, no escribir en lo que tape.
    // 0.1.0
    pub fn toast_pulsado(&mut self, x: f64, y: f64) -> bool {
        let Some((id, _)) = self.toast_bajo(x, y) else {
            return false;
        };
        let descartado =
            self.guard("descartar toast", |host| host.shell.descartar_toast(id)) == Some(true);
        if descartado {
            self.pintar_toasts();
        }
        descartado
    }

    // 0.1.0
    pub fn toast_accion(&mut self, x: f64, y: f64) -> Option<(u32, bookos_shell::toast::Pulsado)> {
        let (id, (tx, ty, _, _)) = self.toast_bajo(x, y)?;
        let pulsado = self
            .guard("acción del toast", |host| {
                let pulsado =
                    host.shell
                        .toast_accion_relativa(id, (x - tx) as f32, (y - ty) as f32);
                if pulsado.is_some() {
                    host.shell.descartar_toast(id);
                }
                pulsado
            })
            .flatten()?;
        self.pintar_toasts();
        Some((id, pulsado))
    }

    /// Retira una notificación por su identificador.
    // 0.1.0
    pub fn cerrar_notificacion(&mut self, id: u32) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("cerrar notificación", |host| {
                host.shell.cerrar_notificacion(id)
            })
            .unwrap_or(false);
        self.tras_notificar(repintar)
    }

    /// Vacía la cola y devuelve a quién hay que avisar por D-Bus.
    // 0.1.0
    pub fn borrar_notificaciones(&mut self) -> Vec<u32> {
        if self.dead {
            return Vec::new();
        }
        let ids = self
            .guard("borrar notificaciones", |host| {
                host.shell.borrar_notificaciones()
            })
            .unwrap_or_default();
        self.tras_notificar(!ids.is_empty());
        ids
    }

    /// Repinta el panel y la tarjeta tras tocar la cola. La chapa del contador
    /// cambia el **ancho** del widget, así que hay que rehacer las zonas de
    /// clic del panel además de su dibujo.
    // 0.1.0
    fn tras_notificar(&mut self, repintar: bool) -> bool {
        if !repintar {
            return false;
        }
        self.guard("pintar panel", |host| {
            let buffer = &mut host.panel.buffer;
            paint(buffer, |buf| host.shell.draw_panel(buf));
        });
        if self.shell.emergente_nombre() == Some("notificaciones") {
            self.sincronizar_emergente();
        }
        true
    }

    /// Repinta el conmutador mientras su recuadro se mueve de celda a celda.
    // 0.1.0
    pub fn animar_conmutador(&mut self) {
        if self.dead || !self.shell.conmutador_animando() {
            return;
        }
        self.guard("animar conmutador", |host| host.pintar_conmutador());
    }

    /// Repinta el dock mientras la placa de un icono entra o sale.
    ///
    /// Va aparte de `refresh` porque el dock no tiene datos que releer: lo que
    /// cambia es solo su dibujo, y `refresh` se llama en el tick del panel —una
    /// vez por minuto— que no es el ritmo de una animación de 120 ms.
    // 0.1.1
    pub fn dock_lanzando(&mut self, exec: &str) -> bool {
        !self.dead
            && self
                .guard("rebote del dock", |host| host.shell.dock_lanzando(exec))
                .unwrap_or(false)
    }

    // 0.1.0
    pub fn animar_dock(&mut self) {
        if self.dead || self.dock_revision == self.shell.dock_revision() {
            return;
        }
        self.guard("pintar capas del dock", |host| {
            let escala = host.shell.scale();
            host.dock_iconos.clear();
            for (i, (_, _, w, h, _)) in host.shell.dock_capas().into_iter().enumerate() {
                let w = bookos_shell::a_pixel_entero(w, escala);
                let h = bookos_shell::a_pixel_entero(h, escala);
                let mut superficie = Surface::new(
                    ((w * escala) as u32, (h * escala) as u32),
                    (w as i32, h as i32),
                );
                paint(&mut superficie.buffer, |buf| {
                    host.shell.draw_dock_icono(i, buf)
                });
                host.dock_iconos.push(superficie);
            }
            host.dock_realces.clear();
            for (_, _, lado, _, _) in host.shell.dock_capas() {
                let lado = bookos_shell::a_pixel_entero(lado, escala);
                let mut superficie = Surface::new(
                    ((lado * escala) as u32, (lado * escala) as u32),
                    (lado as i32, lado as i32),
                );
                paint(&mut superficie.buffer, |buf| {
                    host.shell.draw_dock_realce(buf, lado)
                });
                host.dock_realces.push(superficie);
            }
            host.dock_revision = host.shell.dock_revision();
        });
    }

    // 0.1.0
    pub fn animar_panel(&mut self) {
        if self.dead || !self.shell.panel_animando() {
            return;
        }
        self.guard("animar widgets", |host| {
            host.shell.panel_avanzar();
            paint(&mut host.panel.buffer, |buf| host.shell.draw_panel(buf));
        });
    }

    /// ¿Sigue el aviso a la vista? De paso lo retira si se le acabó el tiempo.
    ///
    /// Devuelve además si **acaba** de retirarlo, porque entonces hace falta un
    /// frame más: el que lo quita de la pantalla.
    // 0.1.0
    pub fn osd_vivo(&mut self) -> (bool, bool) {
        let vivo = self.guard("osd vivo", |host| host.shell.osd_vivo()) == Some(true);
        let retirado = !vivo && self.osd.take().is_some();
        (vivo, retirado)
    }

    /// Cuánto falta para que se vaya, para programar un solo despertar en vez
    /// de repintar sesenta veces por segundo enseñando algo que no cambia.
    // 0.1.0
    pub fn osd_queda(&self) -> Option<std::time::Duration> {
        self.shell.osd_queda()
    }

    /// Repinta el aviso mientras su barra se mueve.
    // 0.1.0
    pub fn animar_osd(&mut self) {
        if self.dead || !self.shell.osd_needs_paint() || self.osd.is_none() {
            return;
        }
        self.guard("animar osd", |host| host.pintar_osd());
    }

    // 0.1.0
    fn pintar_osd(&mut self) {
        let Some((w, h)) = self.shell.osd_buffer_size() else {
            self.osd = None;
            return;
        };
        let Some((lw, lh)) = self.shell.osd_logical_size() else {
            return;
        };
        // Con el aviso ya en pantalla y del mismo tamaño se pinta **sobre su
        // propio buffer**. Crear otro por cada pulsación —mantener la tecla de
        // brillo son decenas por segundo— tiraba la textura y su identidad en
        // el compositor cada vez, y el aviso entero se rehacía en vez de
        // cambiar solo su barra.
        let tam = (w as f64, h as f64).into();
        if let Some(surface) = self.osd.as_mut()
            && surface.src.size == tam
            && surface.logical == (lw as i32, lh as i32).into()
        {
            paint(&mut surface.buffer, |buf| self.shell.draw_osd(buf));
            return;
        }
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Centrado y por encima del dock: taparlo al subir el volumen es justo
        // lo que molesta del OSD de otros escritorios.
        let escala = self.shell.scale() as f64;
        // El buffer empieza más arriba que la cápsula justo lo que ocupa el
        // margen de la sombra; la cuenta la hace el shell, que es donde están
        // las dos medidas.
        let margen = bookos_shell::osd::MARGEN_BUFFER_INFERIOR as f64;
        surface.location = (
            (self.screen.0 as f64 - w as f64) / 2.0,
            self.screen.1 as f64 - margen * escala,
        )
            .into();
        paint(&mut surface.buffer, |buf| self.shell.draw_osd(buf));
        self.osd = Some(surface);
    }

    /// Saca el teclado en pantalla abajo en la principal.
    // 0.1.1
    pub fn mostrar_teclado(&mut self, capa: bookos_shell::teclado::Capa) {
        let ancho = self.area_principal().0 as f32;
        self.guard("mostrar teclado", |host| {
            host.shell.mostrar_teclado(ancho, capa);
            host.pintar_teclado();
        });
    }

    // 0.1.1
    pub fn ocultar_teclado(&mut self) {
        self.shell.ocultar_teclado();
    }

    // 0.1.1
    pub fn teclado_abierto(&self) -> bool {
        !self.dead && self.shell.teclado_abierto()
    }

    /// Lo que tapa el teclado del borde de abajo de la principal, en lógicos.
    // 0.1.1
    pub fn teclado_asoma(&self) -> f64 {
        if self.teclado.is_none() {
            return 0.0;
        }
        self.shell.teclado_asoma() as f64
    }

    /// El punto relativo al teclado si cae encima de él, en lógicos.
    // 0.1.1
    pub fn teclado_en(&self, x: f64, y: f64) -> Option<(f32, f32)> {
        if !self.teclado_abierto() || !self.en_la_principal(x, y) {
            return None;
        }
        let (_, alto) = self.area_principal();
        let arriba = alto - self.teclado_asoma();
        (y >= arriba).then_some((x as f32, (y - arriba) as f32))
    }

    // 0.1.1
    pub fn teclado_pulsar(&mut self, x: f32, y: f32) -> Option<bookos_shell::teclado::Accion> {
        let accion = self.guard("pulsar teclado", |host| host.shell.teclado_pulsar(x, y))?;
        self.guard("pintar teclado", |host| host.pintar_teclado());
        accion
    }

    // 0.1.1
    pub fn teclado_soltar(&mut self) -> Option<bookos_shell::teclado::Accion> {
        let accion = self.guard("soltar teclado", |host| host.shell.teclado_soltar())?;
        self.guard("pintar teclado", |host| host.pintar_teclado());
        accion
    }

    /// Lo tira si ya se fue y lo repinta si cambió algo. Devuelve si hace
    /// falta un fotograma: el que lo quita, o el de un repintado.
    // 0.1.1
    pub fn teclado_tick(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let vivo = self.guard("teclado vivo", |host| host.shell.teclado_vivo()) == Some(true);
        if !vivo {
            return self.teclado.take().is_some();
        }
        if self.shell.teclado_needs_paint() {
            self.guard("pintar teclado", |host| host.pintar_teclado());
            return true;
        }
        false
    }

    /// Repinta el teclado entero: su color sale del tema y del acento.
    // 0.1.1
    pub fn teclado_invalidar(&mut self) {
        self.shell.teclado_invalidar();
    }

    // 0.1.1
    fn pintar_teclado(&mut self) {
        let Some((w, h)) = self.shell.teclado_buffer_size() else {
            self.teclado = None;
            return;
        };
        let Some((lw, lh)) = self.shell.teclado_logical_size() else {
            return;
        };
        // Sobre el buffer que ya hay si es del mismo tamaño, como el aviso: cada
        // pulsación repinta, y tirar la textura en cada una la subía entera.
        let tam = (w as f64, h as f64).into();
        if let Some(surface) = self.teclado.as_mut()
            && surface.src.size == tam
        {
            paint(&mut surface.buffer, |buf| self.shell.draw_teclado(buf));
            return;
        }
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        surface.location = (0.0, self.screen.1 as f64 - h as f64).into();
        paint(&mut surface.buffer, |buf| self.shell.draw_teclado(buf));
        self.teclado = Some(surface);
    }

    /// Cambia la preferencia de efectos y devuelve la elegida, para guardarla.
    // 0.1.0
    pub fn alternar_efectos(&mut self) -> bookos_shell::Efectos {
        self.shell.alternar_efectos()
    }

    /// Recalcula el modo de efectos —lo elegido y lo que impone la batería— y
    /// devuelve si cambió. No pasa por `guard`: no dibuja nada, solo lee sysfs
    /// y mueve un booleano.
    // 0.1.0
    pub fn revisar_efectos(&self) -> bool {
        !self.dead && self.shell.revisar_efectos()
    }

    // --- Panel de diagnóstico -----------------------------------------------

    /// Pone o quita el panel. Devuelve si quedó puesto, que es lo que el
    /// compositor necesita para programar o soltar su despertar por segundo.
    // 0.1.0
    pub fn alternar_diagnostico(&mut self) -> bool {
        let puesto =
            self.guard("diagnóstico", |host| host.shell.alternar_diagnostico()) == Some(true);
        if puesto {
            self.pintar_diagnostico();
        } else {
            self.diagnostico = None;
        }
        puesto
    }

    // 0.1.0
    pub fn diagnostico_visible(&self) -> bool {
        self.diagnostico.is_some()
    }

    /// Entrega la medida de la última ventana. Devuelve si hay que repintar.
    // 0.1.0
    pub fn diagnostico_datos(&mut self, datos: bookos_shell::diagnostico::Datos) -> bool {
        if self.dead || self.diagnostico.is_none() {
            return false;
        }
        let cambio = self.guard("datos del diagnóstico", |host| {
            host.shell.diagnostico_datos(datos)
        }) == Some(true);
        if cambio {
            self.pintar_diagnostico();
        }
        cambio
    }

    // 0.1.0
    // modified 0.1.1 -- repinta sobre su buffer si no cambia de tamaño
    fn pintar_diagnostico(&mut self) {
        let Some((w, h)) = self.shell.diagnostico_buffer_size() else {
            self.diagnostico = None;
            return;
        };
        let Some((lw, lh)) = self.shell.diagnostico_logical_size() else {
            return;
        };
        let escala = self.shell.scale() as f64;
        let margen = 12.0 * escala;
        let location = (
            self.screen.0 as f64 - w as f64 - margen,
            self.shell.panel_height() as f64 * escala + margen,
        )
            .into();
        let tam = (w as f64, h as f64).into();
        if let Some(surface) = self.diagnostico.as_mut()
            && surface.src.size == tam
            && surface.logical == (lw as i32, lh as i32).into()
        {
            surface.location = location;
            paint(&mut surface.buffer, |buf| self.shell.draw_diagnostico(buf));
            return;
        }
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Arriba a la derecha, justo debajo del panel: es la esquina que menos
        // tapa mientras se prueba algo, y no se solapa con el OSD ni con los
        // avisos, que salen abajo y arriba a la derecha respectivamente. El
        // toast se le puede poner encima; se acepta, es una herramienta.
        surface.location = location;
        paint(&mut surface.buffer, |buf| self.shell.draw_diagnostico(buf));
        self.diagnostico = Some(surface);
    }

    // --- Conmutador de aplicaciones ----------------------------------------

    // 0.1.0
    pub fn hay_conmutador(&self) -> bool {
        !self.dead && self.shell.hay_conmutador()
    }

    /// Abre el conmutador. `false` si no había nada que conmutar.
    // 0.1.0
    pub fn abrir_conmutador(
        &mut self,
        modo: bookos_shell::conmutador::Modo,
        apps: Vec<bookos_shell::conmutador::Entrada>,
    ) -> bool {
        if self.dead {
            // Se distingue del `false` normal: un shell muerto tras un panic no
            // vuelve, y confundirlo con "no hay celdas" manda a buscar el fallo
            // al sitio equivocado.
            tracing::warn!("el shell está muerto: no hay conmutador");
            return false;
        }
        let pantalla = (
            self.screen.0 as f32 / self.shell.scale(),
            self.screen.1 as f32 / self.shell.scale(),
        );
        let abierto = self
            .guard("abrir conmutador", |host| {
                host.shell.abrir_conmutador(modo, apps, pantalla)
            })
            .unwrap_or(false);
        if abierto {
            // Otra tarjeta, quizá de otro tamaño: la de antes no vale.
            self.soltar_conmutador();
            self.pintar_conmutador();
        }
        abierto
    }

    // 0.1.0
    fn soltar_conmutador(&mut self) {
        self.conmutador = None;
        self.conmutador_realce = None;
        self.conmutador_rotulo = None;
        self.conmutador_desde = None;
    }

    // 0.1.0
    pub fn conmutador_mover(&mut self, pasos: i32) {
        self.guard("mover conmutador", |host| {
            host.shell.conmutador_mover(pasos)
        });
        self.pintar_conmutador();
    }

    // 0.1.0
    pub fn conmutador_elegir(&mut self, i: usize) -> bool {
        let cambio = self
            .guard("elegir en conmutador", |host| {
                host.shell.conmutador_elegir(i)
            })
            .unwrap_or(false);
        if cambio {
            self.pintar_conmutador();
        }
        cambio
    }

    /// Celda bajo un punto de pantalla lógico.
    // 0.1.0
    pub fn conmutador_en(&self, x: f64, y: f64) -> Option<usize> {
        let surface = self.conmutador.as_ref()?;
        let escala = self.shell.scale() as f64;
        let local_x = x - surface.location.x / escala;
        let local_y = y - surface.location.y / escala;
        self.shell.conmutador_en(local_x as f32, local_y as f32)
    }

    /// Huecos globales lógicos en los que se componen las ventanas vivas.
    // 0.1.0
    pub fn conmutador_miniaturas(&self) -> Vec<Rectangle<i32, Logical>> {
        let Some(surface) = self.conmutador.as_ref() else {
            return Vec::new();
        };
        let escala = self.shell.scale() as f64;
        let origen_x = surface.location.x / escala;
        let origen_y = surface.location.y / escala;
        self.shell
            .conmutador_miniaturas()
            .into_iter()
            .map(|r| {
                Rectangle::new(
                    (
                        (origen_x + r.x as f64).round() as i32,
                        (origen_y + r.y as f64).round() as i32,
                    )
                        .into(),
                    (
                        r.width.round().max(1.0) as i32,
                        r.height.round().max(1.0) as i32,
                    )
                        .into(),
                )
            })
            .collect()
    }

    /// `alfa` es el fundido de la entrada de tres dedos; 1 el resto del tiempo.
    // 0.1.0
    // modified 0.1.1 -- entra con fundido en la exposición
    pub fn conmutador_element<R>(
        &self,
        renderer: &mut R,
        alfa: f32,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        let surface = self.conmutador.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            surface.location,
            &surface.buffer,
            Some(alfa),
            Some(surface.src),
            Some(surface.logical),
            Kind::Unspecified,
        )
        .ok()
    }

    // añadido en la 0.2.0
    pub fn conmutador_elegida(&self) -> Option<usize> {
        self.shell.conmutador_elegida()
    }

    /// Cierra el conmutador y devuelve **qué celda** quedó elegida. La ventana
    /// a la que corresponde la sabe el compositor, no el shell.
    // 0.1.0
    pub fn cerrar_conmutador(&mut self) -> Option<usize> {
        let elegida = self
            .guard("cerrar conmutador", |host| host.shell.cerrar_conmutador())
            .flatten();
        self.soltar_conmutador();
        elegida
    }

    // 0.1.0
    pub fn cancelar_conmutador(&mut self) -> bool {
        let habia = self
            .guard("cancelar conmutador", |host| {
                host.shell.cancelar_conmutador()
            })
            .unwrap_or(false);
        self.soltar_conmutador();
        habia
    }

    /// Rehace su superficie y la centra en la pantalla.
    ///
    /// Se rehace en cada paso y no solo al abrir porque el tamaño no cambia pero
    /// el contenido sí —la celda realzada y el nombre de debajo—, y centrar de
    /// nuevo cuesta dos restas.
    // 0.1.0
    // modified 0.1.1 -- repinta sobre su buffer si no cambia de tamaño
    fn pintar_conmutador(&mut self) {
        let Some((w, h)) = self.shell.conmutador_buffer_size() else {
            self.soltar_conmutador();
            return;
        };
        // El de ventanas pinta la tarjeta una sola vez: lo que cambia al elegir
        // va en el recuadro y el rótulo.
        if self.shell.conmutador_realce().is_some() && self.conmutador.is_some() {
            self.sincronizar_conmutador();
            return;
        }
        let Some((lw, lh)) = self.shell.conmutador_logical_size() else {
            return;
        };
        let location = (
            (self.screen.0 as f64 - w as f64) / 2.0,
            (self.screen.1 as f64 - h as f64) / 2.0,
        )
            .into();
        let tam = (w as f64, h as f64).into();
        if let Some(surface) = self.conmutador.as_mut()
            && surface.src.size == tam
            && surface.logical == (lw as i32, lh as i32).into()
        {
            surface.location = location;
            paint(&mut surface.buffer, |buf| self.shell.draw_conmutador(buf));
            self.sincronizar_conmutador();
            return;
        }
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Centrado en los dos ejes, como el de macOS: es una tarjeta modal, no
        // un aviso que se asoma por un borde.
        surface.location = location;
        paint(&mut surface.buffer, |buf| self.shell.draw_conmutador(buf));
        self.conmutador = Some(surface);
        self.sincronizar_conmutador();
    }

    /// Coloca el recuadro de la elegida y repinta su rótulo.
    ///
    /// El recuadro se pinta una vez y luego solo se mueve —y se desliza, como
    /// el realce del launchpad—. El rótulo sí se repinta, pero mide un renglón.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la superficie del rótulo
    fn sincronizar_conmutador(&mut self) {
        let (Some(celda), Some(renglon)) = (
            self.shell.conmutador_realce(),
            self.shell.conmutador_rotulo(),
        ) else {
            return;
        };
        let Some(origen) = self.conmutador.as_ref().map(|s| s.location) else {
            return;
        };
        let escala = self.shell.scale();
        // Cuadrados al píxel entero con la misma función que `Canvas::new`: con
        // `(w * escala).round()` el buffer no mide lo que espera el shell a
        // escala fraccionaria y el pintado se niega. Ver `sincronizar_realce`.
        let medida = |w: f32, h: f32| {
            let logico = (
                bookos_shell::a_pixel_entero(w, escala),
                bookos_shell::a_pixel_entero(h, escala),
            );
            let fisico = (
                (logico.0 * escala).round().max(1.0) as u32,
                (logico.1 * escala).round().max(1.0) as u32,
            );
            (logico, fisico)
        };
        let colocar = |x: f32, y: f32| {
            Point::<f64, Physical>::from((
                origen.x + (x * escala) as f64,
                origen.y + (y * escala) as f64,
            ))
        };

        let destino = colocar(celda.x, celda.y);
        match self.conmutador_realce.as_ref().map(|s| s.location) {
            Some(antes) => {
                if antes != destino {
                    // Desde donde va **ahora**, no desde su destino anterior:
                    // cruzar tres celdas seguidas no puede dar saltos.
                    self.conmutador_desde =
                        Some((self.conmutador_location(antes), std::time::Instant::now()));
                    if let Some(surface) = self.conmutador_realce.as_mut() {
                        surface.location = destino;
                    }
                }
            }
            None => {
                let (logico, fisico) = medida(celda.width, celda.height);
                let mut surface = Surface::new(fisico, (logico.0 as i32, logico.1 as i32));
                surface.location = destino;
                self.guard("pintar recuadro del conmutador", |host| {
                    paint(&mut surface.buffer, |buf| {
                        host.shell.draw_conmutador_realce(buf, logico.0, logico.1);
                        Vec::new()
                    });
                });
                self.conmutador_realce = Some(surface);
            }
        }

        let (logico, fisico) = medida(renglon.width, renglon.height);
        let tam = (fisico.0 as f64, fisico.1 as f64).into();
        let location = colocar(renglon.x, renglon.y);
        let rehacer = self.conmutador_rotulo.as_ref().is_none_or(|s| {
            s.src.size != tam || s.logical != (logico.0 as i32, logico.1 as i32).into()
        });
        if rehacer {
            self.conmutador_rotulo = Some(Surface::new(fisico, (logico.0 as i32, logico.1 as i32)));
        }
        if let Some(surface) = self.conmutador_rotulo.as_mut() {
            surface.location = location;
        }
        self.guard("pintar rótulo del conmutador", |host| {
            let Some(surface) = host.conmutador_rotulo.as_mut() else {
                return;
            };
            paint(&mut surface.buffer, |buf| {
                host.shell.draw_conmutador_rotulo(buf, logico.0, logico.1);
                // Entero: sin daño, el compositor seguía enseñando el nombre de
                // la ventana anterior aunque el buffer ya llevara el nuevo.
                vec![Damage {
                    x: 0,
                    y: 0,
                    width: fisico.0 as i32,
                    height: fisico.1 as i32,
                }]
            });
        });
    }

    /// Dónde está el recuadro del conmutador **ahora**, contando el deslizamiento.
    /// Misma curva y duración que el realce del launchpad: es un hover.
    // 0.1.0
    fn conmutador_location(&self, destino: Point<f64, Physical>) -> Point<f64, Physical> {
        let Some((desde, t0)) = self.conmutador_desde else {
            return destino;
        };
        use bookos_shell::tema;
        let s = tema::C_SUAVE.eval(tema::fraccion(t0.elapsed(), Self::DESLIZ)) as f64;
        (
            desde.x + (destino.x - desde.x) * s,
            desde.y + (destino.y - desde.y) * s,
        )
            .into()
    }

    /// El recuadro y el rótulo del conmutador de ventanas, para componerlos
    /// entre las miniaturas y la tarjeta.
    // 0.1.0
    // modified 0.1.1 -- entra con fundido en la exposición
    pub fn conmutador_extras<R>(
        &self,
        renderer: &mut R,
        alfa: f32,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        [
            self.conmutador_rotulo.as_ref().map(|s| (s, s.location)),
            self.conmutador_realce
                .as_ref()
                .map(|s| (s, self.conmutador_location(s.location))),
        ]
        .into_iter()
        .flatten()
        .filter_map(|(surface, location)| {
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location,
                &surface.buffer,
                Some(alfa),
                Some(surface.src),
                Some(surface.logical),
                Kind::Unspecified,
            )
            .ok()
        })
        .collect()
    }

    /// Echa el bloqueo y prepara su superficie, del tamaño de la pantalla.
    // 0.1.0
    // modified 0.1.1 -- cancela el arrastre del dock
    pub fn bloquear(&mut self, hora: String, fecha: String, pantalla: (f32, f32)) {
        self.cancelar_arrastre_dock();
        self.guard("bloquear", |host| {
            host.shell.bloquear(hora, fecha, pantalla);
        });
        // modificado en la 0.2.0 -- cada textura nace en su estado final; la entrada no toca sus píxeles.
        self.bloqueo = Some(Vec::new());
        self.repintar_bloqueo(&bookos_shell::bloqueo::Capa::TODAS);
    }

    // añadido en la 0.2.0 -- una tecla invalida acceso; la música y el reloj conservan sus buffers.
    fn repintar_bloqueo(&mut self, pedidas: &[bookos_shell::bloqueo::Capa]) {
        self.guard("pintar capas del bloqueo", |host| {
            let geometria = host.shell.bloqueo_capas();
            let Some(capas) = host.bloqueo.as_mut() else {
                return;
            };
            capas.retain(|(c, _)| geometria.iter().any(|(viva, _)| viva == c));
            let escala = host.shell.scale();
            for (capa, (x, y, w, h)) in geometria {
                let w = bookos_shell::a_pixel_entero(w, escala);
                let h = bookos_shell::a_pixel_entero(h, escala);
                let logico = (w as i32, h as i32);
                let fisico = ((w * escala).round() as u32, (h * escala).round() as u32);
                let indice = match capas.iter().position(|(c, _)| *c == capa) {
                    Some(i) => i,
                    None => {
                        capas.push((capa, Surface::new(fisico, logico)));
                        capas.len() - 1
                    }
                };
                let surface = &mut capas[indice].1;
                let rehacer = surface.logical != logico.into();
                if rehacer {
                    *surface = Surface::new(fisico, logico);
                }
                surface.location = ((x * escala) as f64, (y * escala) as f64).into();
                if rehacer || pedidas.contains(&capa) {
                    paint(&mut surface.buffer, |buf| {
                        host.shell.draw_bloqueo_capa(capa, buf)
                    });
                }
            }
        });
    }

    /// Abre la capa de captura de pantalla y la deja pintada.
    ///
    /// Las superficies se crean aquí y no en `refresh` por lo mismo que la del
    /// bloqueo: tienen que estar puestas en el primer fotograma, o al pulsar
    /// Impr se vería el velo sin la barra durante un frame.
    // 0.1.0
    // modified 0.1.1 -- cancela el arrastre del dock
    pub fn abrir_captura(&mut self) {
        self.cancelar_arrastre_dock();
        if self.dead {
            return;
        }
        let escala = self.shell.scale();
        let pantalla = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        self.guard("abrir captura", |host| {
            host.shell.abrir_captura(pantalla);
        });
        let superficie = |pieza| {
            self.shell.captura_buffer_size(pieza).map(|(w, h)| {
                let logico = (
                    (w as f32 / escala).round() as i32,
                    (h as f32 / escala).round() as i32,
                );
                Surface::new((w, h), logico)
            })
        };
        self.captura = superficie(bookos_shell::PiezaCaptura::Barra);
        self.captura_pastilla = superficie(bookos_shell::PiezaCaptura::Pastilla);
        self.repintar_captura();
    }

    // 0.1.0
    pub fn cerrar_captura(&mut self) -> bool {
        let habia = self
            .guard("cerrar captura", |host| host.shell.cerrar_captura())
            .unwrap_or(false);
        self.captura = None;
        self.captura_pastilla = None;
        habia
    }

    // 0.1.0
    pub fn hay_captura(&self) -> bool {
        !self.dead && self.captura.is_some()
    }

    // 0.1.0
    pub fn captura_animando(&self) -> bool {
        !self.dead && self.shell.captura_animando()
    }

    /// Recoloca las dos piezas y repinta las que lo pidan.
    ///
    /// Recolocar va siempre porque es asignar un punto; repintar, solo si
    /// cambió lo que enseñan. Arrastrando, la barra no se repinta nunca y la
    /// pastilla solo cuando cambian las medidas enteras.
    // 0.1.0
    fn repintar_captura(&mut self) {
        use bookos_shell::PiezaCaptura;
        let escala = self.shell.scale() as f64;
        self.guard("pintar captura", |host| {
            let piezas = [
                (PiezaCaptura::Barra, host.captura.as_mut()),
                (PiezaCaptura::Pastilla, host.captura_pastilla.as_mut()),
            ];
            for (pieza, surface) in piezas {
                let Some(surface) = surface else {
                    continue;
                };
                // Redondeado a píxel entero: el lienzo ya mide píxeles
                // enteros, y colocarlo a medio píxel lo reescalaría borroso.
                if let Some((x, y)) = host.shell.captura_origen(pieza) {
                    surface.location =
                        ((x as f64 * escala).round(), (y as f64 * escala).round()).into();
                }
                if host.shell.captura_needs_paint(pieza) {
                    paint(&mut surface.buffer, |buf| {
                        host.shell.draw_captura(pieza, buf)
                    });
                }
            }
        });
    }

    /// Mueve el puntero sobre la capa, en lógicos de la pantalla.
    // 0.1.0
    pub fn captura_puntero(&mut self, x: f64, y: f64) {
        if !self.hay_captura() {
            return;
        }
        self.guard("puntero de la captura", |host| {
            host.shell.captura_puntero(x as f32, y as f32)
        });
        self.repintar_captura();
    }

    // 0.1.0
    pub fn captura_forma(&self, x: f64, y: f64) -> Option<bookos_shell::captura::Forma> {
        if self.dead {
            return None;
        }
        self.shell.captura_forma(x as f32, y as f32)
    }

    // 0.1.0
    pub fn captura_pulsar(&mut self, x: f64, y: f64) -> Option<bookos_shell::Accion> {
        if !self.hay_captura() {
            return None;
        }
        let accion = self
            .guard("clic en la captura", |host| {
                host.shell.captura_pulsar(x as f32, y as f32)
            })
            .flatten();
        self.repintar_captura();
        accion
    }

    // 0.1.0
    pub fn captura_soltar(&mut self) {
        if !self.hay_captura() {
            return;
        }
        self.guard("soltar en la captura", |host| host.shell.captura_soltar());
        self.repintar_captura();
    }

    /// Una tecla para la capa. Devuelve `(consumida, acción)`.
    // 0.1.0
    pub fn captura_tecla(
        &mut self,
        tecla: bookos_shell::TeclaPulsada,
    ) -> (bool, Option<bookos_shell::Accion>) {
        if !self.hay_captura() {
            return (false, None);
        }
        let resultado = self
            .guard("tecla de la captura", |host| {
                host.shell.captura_tecla(tecla)
            })
            .unwrap_or(bookos_shell::Tecla::Ignorada);
        self.repintar_captura();
        match resultado {
            bookos_shell::Tecla::Cerrar => {
                self.cerrar_captura();
                (true, None)
            }
            bookos_shell::Tecla::Hacer(accion) => (true, Some(accion)),
            bookos_shell::Tecla::Consumida => (true, None),
            bookos_shell::Tecla::Ignorada => (false, None),
        }
    }

    // modificado en la 0.2.0 -- el movimiento solo cambia alfa y posición; se pinta al cambiar contenido.
    pub fn animar_bloqueo(&mut self) -> bool {
        self.sincronizar_velo_bloqueo();
        if self.dead || self.bloqueo.is_none() {
            return false;
        }
        let mut cambio = self
            .shell
            .bloqueo_avanzar_entrada(std::time::Instant::now());
        let animando = self.shell.bloqueo_contenido_animando();
        if animando || self.bloqueo_contenido_animaba {
            self.repintar_bloqueo(&[
                bookos_shell::bloqueo::Capa::Acceso,
                bookos_shell::bloqueo::Capa::Energia,
            ]);
            cambio = true;
        }
        self.bloqueo_contenido_animaba = animando;
        cambio || self.shell.bloqueo_medios_avance() < 1.0
    }

    /// Pone al día el velo cuando cambia el color de la canción. El alfa de
    /// su entrada lo aplica la GPU y escribir la contraseña no lo toca.
    // 0.1.0
    fn sincronizar_velo_bloqueo(&mut self) {
        // Tras desbloquear se deja como está: se desvanece con el bloqueo.
        if self.dead || self.bloqueo.is_none() {
            return;
        }
        let Some(color) = self.shell.bloqueo_velo() else {
            self.bloqueo_velo = None;
            return;
        };
        let clave = [color.r, color.g, color.b, color.a].map(|v| (v * 255.0).round() as u8);
        if self.bloqueo_velo.as_ref().is_some_and(|(_, c)| *c == clave) {
            return;
        }
        // Cien filas bastan: la GPU interpola entre ellas, y un degradado
        // lineal interpolado linealmente sale igual que pintado a 1800 filas.
        const ALTO_VELO: u32 = 100;
        let escala = self.shell.scale();
        let logico = (
            (self.screen.0 as f32 / escala).ceil() as i32,
            (self.screen.1 as f32 / escala).ceil() as i32,
        );
        let mut surface = Surface::new((1, ALTO_VELO), logico);
        paint(&mut surface.buffer, |buf| {
            for (y, px) in buf.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                // Del alfa de arriba a cero al 60 % del alto, como el
                // degradado que pintaba el bloqueo.
                let t = (y as f32 + 0.5) / ALTO_VELO as f32;
                let a = color.a * (1.0 - t / 0.6).max(0.0);
                // Premultiplicado y en B, G, R, A: ver `Surface::new`.
                let byte = |v: f32| (v * a * 255.0).round() as u8;
                px.copy_from_slice(&[byte(color.b), byte(color.g), byte(color.r), byte(1.0)]);
            }
            vec![bookos_shell::Damage {
                x: 0,
                y: 0,
                width: 1,
                height: ALTO_VELO as i32,
            }]
        });
        self.bloqueo_velo = Some((surface, clave));
    }

    /// Un clic sobre el bloqueo, en coordenadas lógicas de la pantalla.
    // 0.1.0
    pub fn bloqueo_pulsado(&mut self, x: f64, y: f64) -> Option<bookos_shell::bloqueo::Peticion> {
        let (peticion, repintar) = self
            .guard("clic en el bloqueo", |host| {
                host.shell.bloqueo_pulsado(x as f32, y as f32)
            })
            .unwrap_or((None, false));
        if repintar {
            self.repintar_bloqueo(&[bookos_shell::bloqueo::Capa::Energia]);
        }
        peticion
    }

    /// Actualiza los puntos y el estado del bloqueo, y lo repinta.
    // 0.1.0
    pub fn bloqueo_estado(&mut self, escritos: usize, estado: bookos_shell::bloqueo::Estado) {
        if self.dead {
            return;
        }
        let cambio = self
            .guard("estado del bloqueo", |host| {
                host.shell.bloqueo_estado(escritos, estado)
            })
            .unwrap_or(false);
        if !cambio {
            return;
        }
        self.repintar_bloqueo(&[bookos_shell::bloqueo::Capa::Acceso]);
    }

    // 0.1.0
    pub fn bloqueo_confirmar_tecla(
        &mut self,
        tecla: bookos_shell::TeclaPulsada,
    ) -> (bool, Option<bookos_shell::bloqueo::Peticion>) {
        self.guard("confirmar en bloqueo", |host| {
            let resultado = host.shell.bloqueo_confirmar_tecla(tecla);
            if resultado.0 {
                host.repintar_bloqueo(&[bookos_shell::bloqueo::Capa::Energia]);
            }
            resultado
        })
        .unwrap_or((true, None))
    }

    // 0.1.0
    pub fn bloqueo_huella(&mut self, huella: bookos_shell::bloqueo::Huella) {
        self.guard("estado de la huella", |host| {
            host.shell.bloqueo_huella(huella);
            host.repintar_bloqueo(&[bookos_shell::bloqueo::Capa::Acceso]);
        });
    }

    // 0.1.0
    pub fn bloqueo_caps_lock(&mut self, activo: bool) {
        if self.dead {
            return;
        }
        let cambio = self
            .guard("Bloq Mayús del bloqueo", |host| {
                host.shell.bloqueo_caps_lock(activo)
            })
            .unwrap_or(false);
        if cambio {
            self.repintar_bloqueo(&[bookos_shell::bloqueo::Capa::Acceso]);
        }
    }

    /// Incorpora la lectura multimedia que llegó en segundo plano y repinta la
    /// capa del bloqueo. Si ya se desbloqueó, el shell la descarta.
    // 0.1.0
    pub fn bloqueo_medio(&mut self, sonando: Option<bookos_shell::medios::Sonando>) {
        if self.dead {
            return;
        }
        let cambio = self
            .guard("medios del bloqueo", |host| {
                host.shell.bloqueo_medio(sonando)
            })
            .unwrap_or(false);
        if !cambio {
            return;
        }
        self.repintar_bloqueo(&[bookos_shell::bloqueo::Capa::Medios]);
    }

    // 0.1.0
    pub fn desbloquear(&mut self) {
        self.shell.desbloquear();
        // La seguridad no espera a la animación: el shell ya está desbloqueado
        // y las ventanas se dibujan desde este fotograma. Lo que se desvanece
        // encima es solo la última imagen del bloqueo.
        self.desbloqueo = self
            .bloqueo
            .take()
            .filter(|_| !bookos_shell::tema::efectos_reducidos())
            .map(|s| (s, std::time::Instant::now()));
        // Sin desvanecido el velo se va con el bloqueo; con él, se desvanece
        // a la vez y lo suelta el final de la animación.
        if self.desbloqueo.is_none() {
            self.bloqueo_velo = None;
        }
    }

    /// Cuánto han entrado las ventanas al desbloquear, de 0 a 1. Empiezan
    /// al 30 % de la salida, cuando el bloqueo ya va por más de la mitad de
    /// su desvanecido. `None` si no se está desbloqueando.
    // 0.1.0
    pub fn progreso_desbloqueo(&self) -> Option<f32> {
        let (_, t0) = self.desbloqueo.as_ref()?;
        let f = bookos_shell::tema::fraccion(t0.elapsed(), Self::dura_desbloqueo());
        // Su propio tramo con su propia curva: recortar un tramo de la curva
        // del bloqueo les dejaba solo la cola, que ya va frenando.
        Some(bookos_shell::tema::C_ENTRADA.eval(((f - 0.3) / 0.7).clamp(0.0, 1.0)))
    }

    /// Lo que dura la salida del bloqueo: la de una página, que es lo que es
    /// —se pasa de una pantalla entera a otra—.
    const DESBLOQUEO: std::time::Duration = bookos_shell::tema::D_DESBLOQUEO;

    /// [`Self::DESBLOQUEO`] con `BOOKOS_ANIM` aplicado, para poder mirarla.
    // 0.1.0
    fn dura_desbloqueo() -> std::time::Duration {
        Self::DESBLOQUEO.mul_f32(Self::lento())
    }

    // 0.1.0
    pub fn esta_bloqueado(&self) -> bool {
        !self.dead && self.shell.esta_bloqueado()
    }

    /// ¿Se está dibujando el dock ahora mismo?
    ///
    /// Con el launchpad abierto depende de la configuración; con cualquier otra
    /// vista a pantalla completa —la captura, el bloqueo— nunca.
    // 0.1.0
    fn dock_a_la_vista(&self) -> bool {
        !self.shell.emergente_tapa_la_pantalla()
            || (self.shell.dock_en_launchpad()
                && self.shell.emergente_nombre() == Some("launchpad"))
    }

    /// ¿Cae el punto dentro del dock?
    // 0.1.0
    pub fn en_el_dock(&self, x: f64, y: f64) -> bool {
        !self.dead
            && self.a_la_vista(Barra::Dock)
            && Self::dentro(Some(self.dock_rect()), x, y).is_some()
    }

    // añadido en la 0.2.0 -- decidir si el toque puede arrastrar no debe activar el control de debajo.
    pub fn toque_arrastrable(&self, x: f64, y: f64) -> bool {
        if self.esta_bloqueado() || self.hay_conmutador() {
            return false;
        }
        if self.hay_captura() {
            return self
                .captura_forma(x, y)
                .is_some_and(|f| f != bookos_shell::captura::Forma::Flecha);
        }
        if self.dock_a_la_vista()
            && self.en_el_dock(x, y)
            && (!self.shell.hay_emergente() || self.shell.emergente_nombre() == Some("launchpad"))
        {
            return true;
        }
        Self::dentro(self.emergente_rect(), x, y)
            .is_some_and(|(x, y)| self.shell.emergente_arrastre_en(x, y))
    }

    // añadido en la 0.2.0 -- cancelar el dispositivo no confirma una aplicación del dock.
    pub fn cancelar_toque(&mut self) {
        self.cancelar_arrastre_dock();
        self.shell.cancelar_arrastre_launchpad();
        self.sincronizar_agarrado();
    }

    /// Abre el menú contextual del icono que haya en ese punto del dock.
    /// `true` si se abrió alguno.
    // 0.1.0
    pub fn menu_dock_en(&mut self, x: f64, y: f64) -> bool {
        let Some((rx, ry)) = Self::dentro(Some(self.dock_rect()), x, y) else {
            return false;
        };
        if self.guard("menú del dock", |host| host.shell.dock_menu(rx, ry)) != Some(true) {
            return false;
        }
        self.sincronizar_emergente();
        true
    }

    // 0.1.0
    pub fn menu_ventana(&mut self, opciones: Vec<(String, Accion)>) {
        self.guard("menú de ventana", |host| host.shell.menu_ventana(opciones));
        self.sincronizar_emergente();
    }

    // 0.1.1
    pub fn menu_escritorio(
        &mut self,
        opciones: Vec<Option<(String, &'static str, Accion)>>,
        punto: (f32, f32),
    ) {
        self.guard("menú del escritorio", |host| {
            host.shell.menu_escritorio(opciones, punto)
        });
        self.sincronizar_emergente();
    }

    /// `true` si el panel cambió de alto: hay que recolocar las ventanas.
    // añadido en la 0.2.0
    pub fn panel_tamano(&mut self, tamano: bookos_shell::TamanoPanel) -> bool {
        if self.dead {
            return false;
        }
        let mut cambio = false;
        self.guard("tamaño del panel", |host| {
            cambio = host.shell.panel_tamano(tamano);
            host.panel = Surface::new(
                host.shell.panel_buffer_size(),
                host.shell.panel_logical_size(),
            );
        });
        if cambio {
            self.escritorio_recolocar();
        }
        cambio
    }

    /// Los widgets elegidos en Ajustes. `true` si el panel cambió.
    ///
    /// No toca el tamaño del buffer: cambiar qué widgets hay no cambia el
    /// alto, y el ancho del panel es el de la pantalla.
    // añadido en la 0.2.0
    pub fn widgets_panel(&mut self, centro: Option<&str>, derecha: &[String]) -> bool {
        if self.dead {
            return false;
        }
        self.guard("widgets del panel", |host| {
            host.shell.widgets_panel(centro, derecha)
        })
        .unwrap_or(false)
    }

    // 0.1.0
    pub fn dock_tamano(&mut self, tamano: u32) -> bool {
        if self.dead || self.shell.dock().tamano_actual() == tamano.clamp(32, 80) {
            return false;
        }
        self.guard("tamaño del dock", |host| host.shell.dock_tamano(tamano));
        self.dock = Surface::new(
            self.shell.dock_buffer_size(),
            self.shell.dock_logical_size(),
        );
        self.place_dock();
        self.avisar_zona_dock();
        self.refresh();
        true
    }

    /// Ancla o desancla una aplicación. Devuelve la lista que hay que guardar.
    // 0.1.0
    pub fn anclar(
        &mut self,
        app_id: &str,
        exec: &str,
        icono: &str,
        fijar: Option<bool>,
    ) -> Option<Vec<String>> {
        let lista = self.guard("anclar", |host| {
            host.shell.anclar(app_id, exec, icono, fijar)
        })?;
        // El dock cambia de tamaño al ganar o perder un icono, así que hay que
        // recolocarlo y volver a pintarlo entero.
        self.dock = Surface::new(
            self.shell.dock_buffer_size(),
            self.shell.dock_logical_size(),
        );
        self.place_dock();
        // El dock acaba de cambiar de ancho, así que la franja que el launchpad
        // tiene por «esto es el dock» ya no vale: se le vuelve a decir. Sin
        // esto, anclar dos aplicaciones seguidas fallaba la segunda, porque el
        // icono se soltaba sobre el dock nuevo y el launchpad seguía midiendo
        // contra el viejo.
        self.avisar_zona_dock();
        Some(lista)
    }

    /// Los primeros elementos visibles del launchpad. Para el autotest.
    // 0.1.0
    pub fn primeros_del_launchpad(&self, n: usize) -> Vec<String> {
        self.shell.launchpad_primeros(n)
    }

    /// El centro de una celda del launchpad, en lógicos **de pantalla**. Para
    /// el autotest, que razona en la misma cuadrícula que el ratón.
    // 0.1.0
    pub fn celda_launchpad(&self, i: usize) -> Option<(f64, f64)> {
        let (ox, oy, _, _) = self.emergente_rect()?;
        self.shell
            .launchpad_celda(i)
            .map(|(x, y)| (ox + x as f64, oy + y as f64))
    }

    /// ¿Está esta aplicación fijada al dock? Para el autotest.
    // 0.1.0
    pub fn esta_anclada(&self, app_id: &str) -> bool {
        !self.dead && self.shell.esta_anclada(app_id)
    }

    /// ¿Se está dibujando el dock? Para el autotest.
    // 0.1.0
    pub fn dock_visible(&self) -> bool {
        !self.dead && self.dock_a_la_vista()
    }

    /// El nombre de la emergente abierta. Para el autotest.
    // 0.1.0
    pub fn emergente_nombre(&self) -> Option<&'static str> {
        (!self.dead)
            .then(|| self.shell.emergente_nombre())
            .flatten()
    }

    /// ¿Hay alguna superficie emergente abierta? Para el autotest.
    // 0.1.0
    pub fn hay_emergente(&self) -> bool {
        !self.dead && self.shell.hay_emergente()
    }

    /// Dónde cae un widget del panel, en lógicos. Para el autotest.
    // 0.1.0
    pub fn zona_widget(&self, nombre: &str) -> Option<(f64, f64)> {
        self.shell
            .zonas_panel()
            .into_iter()
            .find(|(n, _, _)| *n == nombre)
            .map(|(_, x0, x1)| (x0 as f64, x1 as f64))
    }

    /// El botón se ha soltado: lo que estuviera agarrado deja de estarlo.
    /// Clic derecho sobre el launchpad: entra o sale del modo edición, que es
    /// donde cada icono lleva su ✕. `true` si lo ha atendido.
    // 0.1.0
    pub fn launchpad_menu(&mut self) -> bool {
        if self.dead || self.shell.emergente_nombre() != Some("launchpad") {
            return false;
        }
        let hecho =
            self.guard("editar launchpad", |host| host.shell.launchpad_editar()) == Some(true);
        if hecho {
            self.sincronizar_emergente();
        }
        hecho
    }

    /// El botón se ha soltado. Devuelve lo que haya que hacer: soltar un icono
    /// del launchpad sin arrastrarlo es lanzarlo.
    // 0.1.0
    // modified 0.1.1 -- soltar en el dock reordena y guarda; fuera lo quita; sin mover es un clic
    pub fn soltar(&mut self) -> Option<Accion> {
        if self.dead {
            return None;
        }
        if let Some(arrastre) = self.arrastre_burbuja.take() {
            return self.soltar_burbuja(arrastre);
        }
        if let Some(agarrado) = self.arrastre_dock.take() {
            let (x, y) = self.puntero_en;
            let fuera = Self::dentro(Some(self.dock_rect()), x, y).is_none();
            self.reclamar_bajo_el_puntero();
            match agarrado.estado {
                EstadoArrastreDock::Cancelado => return None,
                EstadoArrastreDock::Pulsado => {
                    if fuera {
                        return None;
                    }
                    if !matches!(agarrado.clic, Accion::Launchpad) {
                        self.cerrar_emergente();
                    }
                    return Some(agarrado.clic);
                }
                EstadoArrastreDock::Arrastrando => {
                    self.agarrado = None;
                    self.guard("soltar icono del dock", |host| {
                        host.shell.dock_arrastrar(None);
                        paint(&mut host.dock.buffer, |buf| host.shell.draw_dock(buf));
                    });
                    return Some(if fuera {
                        Accion::Anclar {
                            app_id: agarrado.app_id,
                            exec: agarrado.exec,
                            icono: agarrado.icono,
                            fijar: Some(false),
                        }
                    } else {
                        Accion::GuardarDock(self.shell.dock().como_configuracion())
                    });
                }
            }
        }
        let (repintar, accion) = self
            .guard("soltar", |host| host.shell.soltar())
            .unwrap_or((false, None));
        if repintar || accion.is_some() {
            self.sincronizar_emergente();
        }
        accion
    }

    // 0.1.0
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        if self.dead
            || (self.screen == (width as i32, height as i32) && self.shell.scale() == scale)
        {
            return;
        }
        self.screen = (width as i32, height as i32);
        self.guard("resize", |host| {
            let logical_width = (width as f32 / scale).round().max(1.0) as u32;
            host.shell.resize(logical_width, scale);
            host.shell.panel_pantalla(height as f32 / scale);
            host.panel = Surface::new(
                host.shell.panel_buffer_size(),
                host.shell.panel_logical_size(),
            );
            host.dock = Surface::new(
                host.shell.dock_buffer_size(),
                host.shell.dock_logical_size(),
            );
            host.place_dock();
        });
        self.escritorio_recolocar();
    }

    /// Relee los datos y repinta lo que haya cambiado. Devuelve `true` si hay
    /// algo nuevo en pantalla, es decir, si el compositor tiene que redibujar.
    // 0.1.0
    pub fn no_molestar(&mut self, value: Option<bool>) -> bool {
        self.guard("no_molestar", |host| {
            if let Some(enabled) = value {
                host.shell.poner_no_molestar(enabled);
            }
            host.shell.notificaciones_silenciadas()
        })
        .unwrap_or(false)
    }

    // 0.1.0
    pub fn refresh(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let dirty = self
            .guard("refresh", |host| {
                let mut dirty = false;
                if host.shell.refresh() {
                    let buffer = &mut host.panel.buffer;
                    paint(buffer, |buf| host.shell.draw_panel(buf));
                    dirty = true;
                }
                // El dock es estático: se pinta la primera vez y no se vuelve a
                // tocar hasta que cambie el tamaño de la pantalla.
                if host.shell.dock_needs_paint() {
                    let buffer = &mut host.dock.buffer;
                    paint(buffer, |buf| host.shell.draw_dock(buf));
                    dirty = true;
                }
                dirty
            })
            .unwrap_or(false);

        // La carpeta del escritorio se mira en este mismo latido, que es el que
        // ya despierta al cambiar el minuto: así un fichero nuevo aparece sin
        // abrir un inotify ni programar un temporizador propio. El precio es
        // hasta un minuto de retraso, y es el trato de siempre aquí.
        //
        // Mientras se arrastra no se toca: releer rehace las superficies y el
        // icono se quedaría clavado a media cuesta.
        let escala = self.shell.scale();
        let logica = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        let cambio = !self.escritorio_arrastrando()
            && self
                .guard("releer el escritorio", |host| {
                    host.shell.escritorio_releer(logica)
                })
                .unwrap_or(false);
        if cambio {
            self.escritorio_recolocar();
        }
        if self.shell.emergente_needs_paint() {
            self.sincronizar_emergente();
        }
        // modificado en la 0.2.0 -- el bloqueo también sigue la hora, la batería y la red.
        let bloqueo = self
            .shell
            .bloqueo_mut()
            .is_some_and(|b| b.refrescar(Shell::hora_bloqueo(), Shell::fecha_bloqueo()));
        if bloqueo {
            self.repintar_bloqueo(&[
                bookos_shell::bloqueo::Capa::Reloj,
                bookos_shell::bloqueo::Capa::Energia,
            ]);
        }
        dirty || cambio || bloqueo
    }

    // 0.1.0
    pub fn refresh_emergente(&mut self) -> bool {
        let cambio = self
            .guard("refrescar tarjeta", |host| host.shell.refresh_emergente())
            .unwrap_or(false);
        if cambio {
            self.sincronizar_emergente();
        }
        cambio
    }

    /// Cuánto ha avanzado la animación de entrada, de 0 a 1.
    ///
    /// Las duraciones y las curvas salen de la tabla de movimiento del sistema
    /// de diseño (§2.7), no de aquí: una emergente **es** un popover, y el
    /// sistema le asigna 180 ms con muelle. Antes esto eran 220 y 280 ms con
    /// potencias copiadas del plasmoide de Plasma, que no rebotan; el muelle es
    /// lo que hace que se lea como que aparece y no como que se despliega.
    ///
    /// La animación **no repinta nada**: el buffer está dibujado y lo único que
    /// cambia de un fotograma a otro son el alfa y el tamaño con que el
    /// compositor lo compone, que es trabajo de la GPU.
    // 0.1.0
    // modificado en la 0.2.0 -- alfa y transformación GPU leen la misma posición continua.
    fn animacion(&self) -> (f32, f32, f32) {
        let avance = self.muelle_emergente.get().valor().clamp(0.0, 1.0);
        if self.vista_escritorios_visible() {
            let superficie = self
                .emergente
                .as_ref()
                .expect("la vista visible conserva su superficie");
            // Una franja pegada al borde entra desde fuera de la pantalla;
            // el zoom de las tarjetas dejaría sus miniaturas desalineadas.
            return (avance, 1.0, -superficie.logical.h as f32 * (1.0 - avance));
        }
        // añadido en la 0.2.0 -- el buscador no sale de ningún widget: aparece
        // en su sitio, sin la caída que dice de dónde viene una tarjeta.
        let caida = if self.emergente_mostrada == Some("buscador") {
            0.0
        } else {
            -16.0
        };
        (avance, 0.97 + 0.03 * avance, caida * (1.0 - avance))
    }

    /// Lo que tarda el realce en deslizarse de una celda a otra. Es el token de
    /// hover del sistema: mover el resaltado con el cursor es un hover.
    const DESLIZ: std::time::Duration = bookos_shell::tema::D_HOVER;

    /// ¿Hay una animación en curso? Mientras la haya, el compositor tiene que
    /// seguir dibujando aunque no pase nada más.
    // 0.1.0
    // modified 0.1.1 -- cuenta la animación del teclado en pantalla
    // modificado en la 0.2.0 -- el último movimiento de un arrastre no se queda sin pintar
    pub fn animando(&self) -> bool {
        if self.emergente_por_pintar
            || self.shell.emergente_animando()
            || self.shell.panel_animando()
            || self.shell.dock_animando()
            || self.shell.conmutador_animando()
            || self.shell.osd_animando()
            || self.shell.teclado_animando()
            || self.shell.toasts_animando()
            || self.shell.bloqueo_animando()
            || (self.actividad_visible && self.shell.actividad_animando())
        {
            return true;
        }
        // Las tres condiciones miran si la animación **sigue en marcha**, no si
        // existe. `cerrando_en.is_some()` a secas cerraba un ciclo: la salida no
        // terminaba nunca, `fin_animacion` —que es quien limpia— solo se llama
        // cuando esto devuelve false, y el compositor se quedaba dibujando a
        // toda velocidad para siempre después de cerrar cualquier emergente.
        // Eso era lo que se veía como que el escritorio iba a tirones, y no
        // paraba hasta reiniciar la sesión.
        (self.launchpad_gesto.is_none()
            && self.vista_gesto.is_none()
            && !self.muelle_emergente.get().quieto())
            || self
                .pagina_anterior
                .as_ref()
                .is_some_and(|(_, t, _)| t.animando())
            || self
                .vista_anterior
                .as_ref()
                .is_some_and(|(_, t, _)| t.animando())
            || self
                .realce_desde
                .is_some_and(|(_, t)| t.elapsed() < Self::DESLIZ)
            || self
                .conmutador_desde
                .is_some_and(|(_, t)| t.elapsed() < Self::DESLIZ)
            || self
                .desbloqueo
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed() < Self::dura_desbloqueo())
    }

    /// El velo a pantalla completa que pide la emergente, si lo pide.
    // 0.1.0
    pub fn velo(
        &self,
    ) -> Option<smithay::backend::renderer::element::solid::SolidColorRenderElement> {
        use smithay::backend::renderer::element::solid::SolidColorRenderElement;
        use smithay::backend::renderer::element::{Id, Kind};

        if self.dead {
            return None;
        }
        // El identificador tiene que ser **el mismo** en cada frame: el damage
        // tracker compara elementos por id, y uno nuevo cada vez le diría que
        // toda la pantalla ha cambiado.
        static ID: std::sync::OnceLock<Id> = std::sync::OnceLock::new();

        let [r, g, b, a] = self.shell.emergente_velo()?;
        let a = a * self.animacion().0;
        Some(SolidColorRenderElement::new(
            ID.get_or_init(Id::new).clone(),
            Rectangle::new((0, 0).into(), (self.screen.0, self.screen.1).into()),
            0,
            // Premultiplicado, que es lo que espera el renderer.
            [r * a, g * a, b * a, a],
            Kind::Unspecified,
        ))
    }

    /// La barra y la pastilla de la capa de captura, que van delante de su
    /// velo.
    ///
    /// Por delante de todo lo demás del shell menos el bloqueo: la capa tapa el
    /// escritorio entero, y el panel o el dock por encima saldrían en la foto
    /// que se está encuadrando.
    // 0.1.0
    pub fn captura_elements<R>(&self, renderer: &mut R) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead || self.bloqueo.is_some() {
            return Vec::new();
        }
        let hay_recuadro = self.shell.captura_marcado().is_some();
        self.captura_pastilla
            .iter()
            .filter(|_| hay_recuadro)
            // La barra de modos se aparta mientras se arrastra: está en medio de
            // la pantalla, y al marcar una zona que pasa por debajo tapaba justo
            // lo que se quería capturar. Vuelve al soltar. La pastilla de
            // medidas sí se queda, que es lo que se mira mientras se arrastra.
            .chain(
                self.captura
                    .iter()
                    .filter(|_| !self.shell.captura_animando()),
            )
            .filter_map(|surface| {
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    surface.location,
                    &surface.buffer,
                    None,
                    Some(surface.src),
                    Some(surface.logical),
                    Kind::Unspecified,
                )
                .inspect_err(|_| tracing::warn!("no se pudo subir una textura de la captura"))
                .ok()
            })
            .collect()
    }

    /// Lo que necesita el velo de la captura: la pantalla en físicos, la
    /// escala y el recuadro marcado. `None` si no hay velo que poner.
    // 0.1.0
    pub fn datos_velo_captura(
        &self,
    ) -> Option<((i32, i32), f64, Option<bookos_shell::captura::Recuadro>)> {
        if self.dead || self.bloqueo.is_some() || self.captura.is_none() {
            return None;
        }
        Some((
            self.screen,
            self.shell.scale() as f64,
            self.shell.captura_marcado(),
        ))
    }

    /// El velo de la capa de captura con esquinas rectas: cuatro bandas
    /// alrededor del recuadro y los cuatro lados del marco, o una sola banda a
    /// pantalla completa si no hay nada marcado.
    ///
    /// Es el camino de repuesto para cuando el driver no compila el shader de
    /// [`crate::captura::Velo`], que es el que redondea. Son rectángulos de
    /// color y no un buffer del shell por lo mismo que la vista previa del
    /// encaje: cambian con cada movimiento del ratón, y rasterizarlos en CPU a
    /// pantalla completa costaba 24,5 ms por evento.
    // 0.1.0
    pub fn velo_captura(
        &self,
    ) -> Vec<smithay::backend::renderer::element::solid::SolidColorRenderElement> {
        use bookos_shell::captura::{BORDE, VELO};
        use smithay::backend::renderer::element::Id;
        use smithay::backend::renderer::element::solid::SolidColorRenderElement;

        let Some(((sw, sh), escala, marcado)) = self.datos_velo_captura() else {
            return Vec::new();
        };
        // Un identificador estable por rectángulo: el damage tracker compara
        // elementos por id, y uno nuevo cada vez le diría que ha cambiado la
        // pantalla entera.
        static IDS: std::sync::OnceLock<[Id; 8]> = std::sync::OnceLock::new();
        let ids = IDS.get_or_init(|| std::array::from_fn(|_| Id::new()));

        // Premultiplicado, que es lo que espera el renderer: el negro se queda
        // en ceros y solo lleva alfa.
        let velo = [0.0, 0.0, 0.0, VELO];
        let rect = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());

        let rects = match marcado {
            None => vec![(rect(0, 0, sw, sh), velo)],
            Some(r) => {
                let fisico = |v: f32| (v as f64 * escala).round() as i32;
                let (x0, y0) = (fisico(r.x), fisico(r.y));
                let (x1, y1) = (fisico(r.x + r.w), fisico(r.y + r.h));
                let (w, h) = (x1 - x0, y1 - y0);
                let b = fisico(BORDE).max(1);
                let acento = bookos_shell::tema::acento();
                let marco = [acento.r, acento.g, acento.b, 1.0];
                // El marco va **dentro** del recuadro, como lo dibujaba el
                // borde de iced: lo que queda bajo la línea sí sale en la foto.
                vec![
                    (rect(0, 0, sw, y0), velo),
                    (rect(0, y1, sw, sh - y1), velo),
                    (rect(0, y0, x0, h), velo),
                    (rect(x1, y0, sw - x1, h), velo),
                    (rect(x0, y0, w, b), marco),
                    (rect(x0, y1 - b, w, b), marco),
                    (rect(x0, y0, b, h), marco),
                    (rect(x1 - b, y0, b, h), marco),
                ]
            }
        };
        rects
            .into_iter()
            .zip(ids)
            .filter(|((r, _), _)| r.size.w > 0 && r.size.h > 0)
            .map(|((r, color), id)| {
                SolidColorRenderElement::new(id.clone(), r, 0, color, Kind::Unspecified)
            })
            .collect()
    }

    /// Los elementos que se le pasan a `render_output` como `custom_elements`.
    /// `capa` deja pedir la isla aparte del resto: ver [`CapaShell`].
    /// Con `completa` —una ventana a pantalla completa en la salida— no van
    /// el panel, el dock ni la isla, que son del escritorio y taparían el
    /// vídeo. Lo demás sí: la captura, el launchpad, una emergente abierta
    /// con su atajo, el OSD. Antes se descartaba todo el shell, y con un vídeo
    /// a pantalla completa no salía ni la capa de captura.
    // 0.1.0
    // modified 0.1.1 -- teclado en pantalla, deslizamiento lateral de avisos e icono agarrado a la escala de su salida
    pub fn elements<R>(
        &self,
        renderer: &mut R,
        completa: bool,
        capa: CapaShell,
        escala_salida: f64,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead {
            return Vec::new();
        }
        // Orden de delante hacia atrás: el aviso por encima de todo, luego la
        // emergente, su realce justo detrás —para que quede bajo los iconos,
        // que van en el buffer de la emergente— y al fondo el panel y el dock.
        //
        // Cada superficie lleva su papel escrito y no se deduce de la posición
        // en la lista. Antes se decidía por índice y el aviso, que va el
        // primero, se llevaba el alfa y el zoom de la emergente mientras la
        // emergente se quedaba sin animar.
        #[derive(Clone, Copy)]
        enum Papel {
            /// Se desvanece con su propio reloj.
            Aviso,
            /// Sube desde el borde de abajo.
            Teclado,
            /// Entra y sale con la animación de la emergente.
            Emergente,
            /// Página del Launchpad que sale por el lado contrario.
            PaginaAnterior(f32, f32),
            /// La vista del launchpad que se está yendo: la rejilla al abrir
            /// una carpeta, el panel al cerrarla, lo de antes en un fundido.
            VistaAnterior(f32, bookos_shell::TransicionLaunchpad),
            /// El aviso de una notificación, arriba a la derecha, y cuánto
            /// baja dentro de la pila.
            Toast(u32, f32),
            /// Actividad viva que emerge del borde superior.
            Actividad,
            /// Igual, y además se desliza al cambiar de fila.
            Realce,
            /// Se aparta por su borde al esquivar ventanas.
            Barra(Barra),
            // añadido en la 0.2.0 -- cada textura comparte la ocultación de la barra y añade su desplazamiento propio.
            IconoDock(usize, f32),
            RealceDock(usize, f32),
            /// El bloqueo que se va: se desvanece y se acerca un poco, como si
            /// se levantara, dejando ver el escritorio.
            Desbloqueo(f32),
            // añadido en la 0.2.0 -- la GPU compone el contenido ya rasterizado.
            Bloqueo(f32),
            VeloBloqueo(f32),
            /// Quieto donde está, opaco: el panel de diagnóstico. No se anima a
            /// propósito — una herramienta de medida que entra con un muelle
            /// falsearía los primeros fotogramas de lo que se está midiendo.
            Fijo,
            Agarrado,
        }

        // La primera vez que sale en un fotograma es cuando empieza a verse:
        // ahí arranca la entrada, y no cuando se creó la superficie.
        if self.entrada_sin_ver.replace(false) && self.cerrando_en.is_none() {
            let mut muelle = self.muelle_emergente.get();
            muelle.fijar(0.0);
            muelle.ir_a(1.0);
            self.muelle_emergente.set(muelle);
        }
        let (alfa_emergente, zoom, y_emergente) = self.animacion();
        // Lo que lleva recorrido la transición de vista del launchpad, si hay
        // una en marcha. La misma cuenta que la página: fracción de tiempo por
        // la curva que le toque.
        let vista = self
            .vista_anterior
            .as_ref()
            .map(|(_, avance, t)| (avance.valor(), *t));
        let pagina = self
            .pagina_anterior
            .as_ref()
            .map(|(_, avance, hacia)| (avance.valor().clamp(0.0, 1.0), *hacia));
        let alfa_osd = self.shell.osd_alfa();
        let escala_osd = self.shell.osd_escala();
        // Con el bloqueo echado **solo** se dibuja el bloqueo, y el aviso de
        // volumen o brillo por encima: el panel y el dock delatarían lo que hay
        // detrás —y el dock, además, deja pulsar iconos—. Esta superficie
        // faltaba en la lista: se creaba y se pintaba, pero no llegaba a la
        // escena, así que Meta+L escondía las ventanas y no enseñaba nada.
        let bloqueado = self.bloqueo.is_some();
        let desplazamientos_toast = self.toasts_desplazamientos();
        let desbloqueo = self.progreso_desbloqueo();
        let iconos_dock = self.shell.dock_capas();
        let superficies = self
            .desbloqueo
            .iter()
            .flat_map(|(capas, t0)| {
                capas.iter().rev().map(move |(_, s)| {
                    let t = bookos_shell::tema::C_SUAVE.eval(bookos_shell::tema::fraccion(
                        t0.elapsed(),
                        Self::dura_desbloqueo(),
                    ));
                    (s, Papel::Desbloqueo(t))
                })
            })
            .chain(self.bloqueo.iter().flat_map(|capas| {
                capas.iter().rev().map(|(capa, s)| {
                    let avance = self.shell.bloqueo_entradas()[*capa as usize];
                    let medios = if *capa == bookos_shell::bloqueo::Capa::Medios {
                        self.shell.bloqueo_medios_avance()
                    } else {
                        1.0
                    };
                    (s, Papel::Bloqueo(avance * medios))
                })
            }))
            // El velo de la canción, detrás del bloqueo y con su mismo papel:
            // fijo mientras está echado y desvaneciéndose con él al quitarlo.
            .chain(self.bloqueo_velo.iter().map(|(s, _)| {
                let papel = match &self.desbloqueo {
                    Some((_, t0)) => Papel::Desbloqueo(bookos_shell::tema::C_SUAVE.eval(
                        bookos_shell::tema::fraccion(t0.elapsed(), Self::dura_desbloqueo()),
                    )),
                    None => Papel::VeloBloqueo(self.shell.bloqueo_medios_avance()),
                };
                (s, papel)
            }))
            // La capa de captura no va en esta lista: su velo es de color
            // sólido y tiene que quedar entre ella y todo lo de aquí, así que
            // la pone la escena con `captura_elements` y `velo_captura`.
            .chain(self.osd.iter().map(|s| (s, Papel::Aviso)))
            // Por delante de las ventanas, el dock y las emergentes: mientras
            // está, es lo que se está usando. Con el bloqueo, también: el
            // campo de la contraseña es un campo.
            .chain(self.teclado.iter().map(|s| (s, Papel::Teclado)))
            // Con el bloqueo echado tampoco sale el diagnóstico: la regla es
            // que ahí solo se dibuja el bloqueo, y una excepción «porque esta
            // no enseña nada» es como se erosionan las reglas de ese tipo.
            .chain(
                self.diagnostico
                    .iter()
                    .map(|s| (s, Papel::Fijo))
                    .filter(|_| !bloqueado),
            )
            .chain(desplazamientos_toast.iter().filter_map(|(id, dy)| {
                let (_, s) = self.toasts.iter().find(|(t, _)| t == id)?;
                Some((s, Papel::Toast(*id, *dy)))
            }))
            .chain(
                self.actividad
                    .iter()
                    .map(|s| (s, Papel::Actividad))
                    .filter(|_| !bloqueado && !completa && self.actividad_visible),
            )
            .chain(
                self.pagina_anterior
                    .iter()
                    .map(|(s, t, hacia)| {
                        (s, Papel::PaginaAnterior(t.valor().clamp(0.0, 1.0), *hacia))
                    })
                    .filter(|_| !bloqueado),
            )
            .chain(
                self.vista_anterior
                    .iter()
                    .map(|(s, avance, t)| (s, Papel::VistaAnterior(avance.valor(), *t)))
                    .filter(|_| !bloqueado),
            )
            // Lo que la mano lleva va por delante de todo lo del shell: es lo
            // único que se está moviendo, y pasar por encima del dock es parte
            // del gesto de anclar.
            .chain(
                self.agarrado
                    .iter()
                    .map(|s| (s, Papel::Agarrado))
                    .filter(|_| !bloqueado),
            )
            // El dock **por delante** del launchpad, y solo ahí. El primero de
            // esta cadena es el que queda más cerca del usuario, así que con el
            // dock en su sitio de siempre —el final— la rejilla se lo comía:
            // la superficie estaba en la lista y no se veía ni un píxel.
            // Comprobado con `BOOKOS_SELFTEST_ANCLAR=captura`.
            .chain(
                self.dock_iconos
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (s, Papel::IconoDock(i, iconos_dock[i].4)))
                    .chain(
                        self.shell
                            .dock_realces()
                            .into_iter()
                            .filter_map(|(i, alfa)| {
                                self.dock_realces
                                    .get(i)
                                    .map(|s| (s, Papel::RealceDock(i, alfa)))
                            }),
                    )
                    .chain(std::iter::once((&self.dock, Papel::Barra(Barra::Dock))))
                    .filter(|_| {
                        !bloqueado
                            && !completa
                            && self.captura.is_none()
                            && self.shell.emergente_tapa_la_pantalla()
                            && self.dock_a_la_vista()
                    }),
            )
            // El realce va detrás de la emergente para quedar bajo los iconos
            // —van en su buffer—, salvo cuando la emergente pide lo contrario:
            // la carpeta abierta del launchpad dibuja un panel opaco, y detrás
            // de él el realce no se vería.
            .chain(
                self.realce
                    .iter()
                    .map(|s| (s, Papel::Realce))
                    .filter(|_| !bloqueado && pagina.is_none() && self.shell.realce_delante()),
            )
            .chain(
                self.emergente
                    .iter()
                    .map(|s| (s, Papel::Emergente))
                    .filter(|_| !bloqueado),
            )
            .chain(
                self.realce
                    .iter()
                    .map(|s| (s, Papel::Realce))
                    .filter(|_| !bloqueado && pagina.is_none() && !self.shell.realce_delante()),
            )
            // Con una vista a pantalla completa no se dibujan las barras: el
            // panel por encima delataría que lo de debajo sigue ahí.
            .chain(
                [(&self.panel, Papel::Barra(Barra::Panel))]
                    .into_iter()
                    .filter(|_| {
                        !bloqueado
                            && !completa
                            && self.captura.is_none()
                            && !self.shell.emergente_tapa_la_pantalla()
                    }),
            )
            // Y aquí el dock de siempre, por detrás de las emergentes: un menú
            // del clic derecho tiene que salir **encima** del icono que lo
            // abrió. Con el launchpad abierto ya se ha puesto arriba.
            .chain(
                self.dock_iconos
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (s, Papel::IconoDock(i, iconos_dock[i].4)))
                    .chain(
                        self.shell
                            .dock_realces()
                            .into_iter()
                            .filter_map(|(i, alfa)| {
                                self.dock_realces
                                    .get(i)
                                    .map(|s| (s, Papel::RealceDock(i, alfa)))
                            }),
                    )
                    .chain(std::iter::once((&self.dock, Papel::Barra(Barra::Dock))))
                    .filter(|_| {
                        !bloqueado
                            && !completa
                            && self.captura.is_none()
                            && !self.shell.emergente_tapa_la_pantalla()
                    }),
            );

        superficies
            .filter(|(_, papel)| match capa {
                CapaShell::Todo => true,
                CapaShell::SinIsla => !matches!(papel, Papel::Actividad),
                CapaShell::SoloIsla => matches!(papel, Papel::Actividad),
            })
            .filter_map(|(surface, papel)| {
                let (alfa, zoom, desplazamiento_x, desplazamiento_y) = match papel {
                    Papel::Aviso => (alfa_osd, escala_osd, 0.0, 0.0),
                    Papel::Teclado => (
                        1.0,
                        1.0,
                        0.0,
                        surface.logical.h as f32 - self.shell.teclado_asoma(),
                    ),
                    Papel::Toast(id, dy) => match self.shell.toast_estado(id) {
                        Some((alfa, zoom, _)) => (
                            alfa,
                            zoom,
                            self.shell
                                .toast_desplazamiento_horizontal(id)
                                .unwrap_or(0.0),
                            dy,
                        ),
                        None => (0.0, 1.0, 0.0, dy),
                    },
                    Papel::Emergente => match (pagina, vista) {
                        (Some((t, hacia)), _) => (t, 1.0, hacia * 140.0 * (1.0 - t), 0.0),
                        // La vista que **entra**. El panel de una carpeta nace
                        // al 94 % y crece con el muelle; al cerrar, la rejilla
                        // vuelve desde el 97 % y sin él.
                        (None, Some((t, cual))) => {
                            use bookos_shell::TransicionLaunchpad as T;
                            match cual {
                                T::AbrirDetalle(origen) => Self::geometria_detalle(
                                    (surface.logical.w as f32, surface.logical.h as f32),
                                    (origen.x, origen.y, origen.width, origen.height),
                                    t,
                                    false,
                                ),
                                T::AbrirCarpeta => (t.clamp(0.0, 1.0), 0.94 + 0.06 * t, 0.0, 0.0),
                                T::CerrarCarpeta => (t.clamp(0.0, 1.0), 0.97 + 0.03 * t, 0.0, 0.0),
                                T::Fundido | T::CerrarDetalle(_) => {
                                    (t.clamp(0.0, 1.0), 1.0, 0.0, 0.0)
                                }
                            }
                        }
                        (None, None) => (alfa_emergente, zoom, 0.0, y_emergente),
                    },
                    Papel::PaginaAnterior(t, hacia) => (1.0 - t, 1.0, -hacia * 140.0 * t, 0.0),
                    Papel::VistaAnterior(t, cual) => {
                        use bookos_shell::TransicionLaunchpad as T;
                        // La rejilla se aparta al abrir —se aleja al 96 %, que
                        // es lo que dice que la carpeta está DENTRO de ella— y
                        // el panel se encoge hacia su tapa al cerrarse.
                        match cual {
                            T::CerrarDetalle(origen) => Self::geometria_detalle(
                                (surface.logical.w as f32, surface.logical.h as f32),
                                (origen.x, origen.y, origen.width, origen.height),
                                t,
                                true,
                            ),
                            T::AbrirCarpeta => {
                                ((1.0 - t).clamp(0.0, 1.0), 1.0 - 0.04 * t, 0.0, 0.0)
                            }
                            T::CerrarCarpeta => {
                                ((1.0 - t).clamp(0.0, 1.0), 1.0 - 0.06 * t, 0.0, 0.0)
                            }
                            T::AbrirDetalle(_) => {
                                // El fundido simétrico superponía los dos textos
                                // hasta media transición en la captura a cámara lenta.
                                ((1.0 - 2.0 * t).clamp(0.0, 1.0), 1.0, 0.0, 0.0)
                            }
                            T::Fundido => ((1.0 - t).clamp(0.0, 1.0), 1.0, 0.0, 0.0),
                        }
                    }
                    Papel::Realce => (alfa_emergente, zoom, 0.0, y_emergente),
                    // La isla nace dentro de la franja del panel y se va con
                    // él: con `panel_esquiva` y una ventana maximizada se
                    // quedaba sola en y=3, tapando el centro de la cabecera de
                    // la ventana —en el calendario, «Día · Semana · Mes · Año»—.
                    //
                    // Subirla el alto del panel no basta: su sombra cuelga 26 px
                    // por debajo de la píldora y quedaba una mancha en el borde.
                    // Por eso además se desvanece al mismo ritmo.
                    // Zenith tampoco, ni en píldora ni en burbuja: ver `isla_sin_panel`.
                    Papel::Actividad if self.isla_sin_panel() => {
                        let (a, z, x, y) = self.shell.actividad_entrada();
                        (a, z, x, y)
                    }
                    Papel::Actividad => {
                        let (a, z, x, y) = self.shell.actividad_entrada();
                        let fuera = self.desplazamiento(Barra::Panel) / self.shell.scale() as f64;
                        let visible = 1.0 - self.barra(Barra::Panel).fraccion();
                        (a * visible, z, x, y - fuera as f32)
                    }
                    // El bloqueo se va en el primer 55 % y las ventanas
                    // entran a partir del 30 % (ver `progreso_desbloqueo`).
                    // A la vez, a mitad de camino se veían las dos capas de
                    // texto al 50 % una encima de otra (capturado con
                    // BOOKOS_ANIM=8): ilegible.
                    Papel::VeloBloqueo(avance) => (avance, 1.0, 0.0, 0.0),
                    Papel::Bloqueo(avance) => (avance, 1.0, 0.0, (1.0 - avance) * 22.0),
                    Papel::Desbloqueo(t) => {
                        let t = (t * 1.8).min(1.0);
                        (1.0 - t, 1.0 + 0.05 * t, 0.0, 0.0)
                    }
                    // Al desbloquear, panel y dock llegan con el escritorio,
                    // como la barra de menús y el Dock de macOS: antes salían
                    // enteros en el primer fotograma, antes que las ventanas.
                    Papel::Barra(_) | Papel::IconoDock(_, _) => {
                        (desbloqueo.unwrap_or(1.0), 1.0, 0.0, 0.0)
                    }
                    Papel::RealceDock(_, alfa) => (alfa * desbloqueo.unwrap_or(1.0), 1.0, 0.0, 0.0),
                    Papel::Fijo | Papel::Agarrado => (1.0, 1.0, 0.0, 0.0),
                };
                // El zoom crece desde el centro, así que hay que compensar la
                // posición: si no, la superficie se agranda hacia la derecha y
                // hacia abajo y parece que se desliza en diagonal.
                let escala = if matches!(papel, Papel::Agarrado) {
                    escala_salida
                } else {
                    f64::from(self.shell.scale())
                };
                let ancho = surface.logical.w as f64 * escala;
                let alto = surface.logical.h as f64 * escala;
                let base = match papel {
                    Papel::Agarrado => self.agarrado_posicion.to_physical(escala),
                    Papel::Realce => self.realce_location(surface.location),
                    Papel::IconoDock(i, salto) | Papel::RealceDock(i, salto) => {
                        let (x, y, _, _, _) = iconos_dock[i];
                        let salto = if matches!(papel, Papel::IconoDock(_, _)) {
                            salto
                        } else {
                            0.0
                        };
                        let llegada = (1.0 - f64::from(desbloqueo.unwrap_or(1.0))) * 10.0 * escala;
                        Point::from((
                            self.dock.location.x + x as f64 * escala,
                            self.dock.location.y
                                + (y - salto) as f64 * escala
                                + self.desplazamiento(Barra::Dock)
                                + llegada,
                        ))
                    }
                    Papel::Barra(cual) => {
                        // El panel se aparta hacia arriba y el dock hacia abajo.
                        let signo = match cual {
                            Barra::Panel => -1.0,
                            Barra::Dock => 1.0,
                        };
                        // Y entran desde su borde, 10 lógicos: lo justo para que
                        // se lea «vuelve a su sitio» sin que parezca que se
                        // esconden.
                        let llegada = (1.0 - f64::from(desbloqueo.unwrap_or(1.0))) * 10.0 * escala;
                        Point::from((
                            surface.location.x,
                            surface.location.y + signo * (self.desplazamiento(cual) + llegada),
                        ))
                    }
                    _ => Point::from((
                        surface.location.x + desplazamiento_x as f64 * escala,
                        surface.location.y + desplazamiento_y as f64 * escala,
                    )),
                };
                // modified 0.2.0 -- tamaño redondeado en vez de truncado, y centrado con el
                // tamaño ya redondeado. El muelle se acerca a 1 sin llegar (0,9995…) y
                // `as i32` lo dejaba en w−1 todo ese tramo final; al asentarse `zoom`
                // valía 1,0 exacto y la tarjeta saltaba 1 lógico (1,75 físicos a la
                // escala de este portátil). Redondeando, el tramo final ya mide w.
                let logical = Size::<i32, Logical>::from((
                    (surface.logical.w as f32 * zoom).round() as i32,
                    (surface.logical.h as f32 * zoom).round() as i32,
                ));
                let location = Point::<f64, Physical>::from((
                    base.x - ancho * (logical.w as f64 / surface.logical.w as f64 - 1.0) / 2.0,
                    base.y - alto * (logical.h as f64 / surface.logical.h as f64 - 1.0) / 2.0,
                ));
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    location,
                    &surface.buffer,
                    Some(alfa),
                    Some(surface.src),
                    Some(logical),
                    Kind::Unspecified,
                )
                .inspect_err(|_| tracing::warn!("no se pudo subir una textura del shell"))
                .ok()
            })
            .collect()
    }

    /// Ejecuta `f` atrapando cualquier panic del shell.
    ///
    /// `AssertUnwindSafe` es correcto aquí porque el único estado que puede
    /// quedar a medias es el del propio shell, y al marcarlo muerto no se
    /// vuelve a tocar.
    // 0.1.0
    fn guard<T>(&mut self, que: &str, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
        // modificado en la 0.2.0 -- incluye leer aplicaciones y construir la tarjeta, antes de rasterizarla.
        let anterior = self.shell.emergente_nombre();
        let pedida = std::time::Instant::now();
        match catch_unwind(AssertUnwindSafe(|| f(self))) {
            Ok(value) => {
                if let Some(nombre) = self
                    .shell
                    .emergente_nombre()
                    .filter(|n| Some(*n) != anterior)
                {
                    APERTURAS.with(|aperturas| {
                        let mut aperturas = aperturas.borrow_mut();
                        if !aperturas.iter().any(|(n, _)| *n == nombre) {
                            aperturas.push((nombre, pedida));
                        }
                    });
                }
                Some(value)
            }
            Err(_) => {
                tracing::error!(
                    que,
                    "el shell ha entrado en panic; se desactiva y la sesión \
                     continúa sin panel ni dock"
                );
                self.dead = true;
                None
            }
        }
    }
}

/// Vuelca un dibujo del shell en su `MemoryRenderBuffer`, traduciendo los
/// rectángulos dañados de iced a los de Smithay.
// 0.1.0
// modificado en la 0.2.0 -- cronometra el embudo de pintura completo.
fn paint(buffer: &mut MemoryRenderBuffer, draw: impl FnOnce(&mut [u8]) -> Vec<Damage>) {
    let desde = std::time::Instant::now();
    let mut ctx = buffer.render();
    let result = ctx.draw(|slice| {
        let damage = draw(slice);
        Ok::<_, std::convert::Infallible>(
            damage
                .into_iter()
                .map(|r| Rectangle::new((r.x, r.y).into(), (r.width, r.height).into()))
                .collect(),
        )
    });
    // El error es Infallible, pero dejarlo explícito documenta que aquí no se
    // traga nada en silencio.
    if let Err(err) = result {
        match err {}
    }
    COSTE_PINTURA.with(|coste| coste.set(coste.get() + desde.elapsed()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn el_launchpad_conserva_el_cristal_sobre_una_ventana_a_pantalla_completa() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        assert!(host.zonas_cristal(true).is_empty());

        host.alternar_launchpad();
        let zonas = host.zonas_cristal(true);
        assert_eq!(zonas.len(), 1, "el launchpad desenfoca toda la pantalla");
        assert_eq!(
            zonas[0].0,
            smithay::utils::Rectangle::from_size((1920, 1080).into())
        );
        assert_eq!(zonas, host.zonas_cristal(false));

        host.cerrar_emergente();
        assert!(
            host.zonas_cristal(true).is_empty(),
            "el cristal no queda sobre el vídeo"
        );
    }

    fn centro_icono_dock(host: &super::ShellHost, indice: usize) -> (f64, f64) {
        let (x, y, _, alto) = host.dock_rect();
        (
            x + f64::from(host.shell.dock().centro_de(indice)),
            y + alto / 2.0,
        )
    }

    #[test]
    fn el_arrastre_conserva_el_fondo_del_dock() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        let inicio = centro_icono_dock(&host, 1);
        let destino = centro_icono_dock(&host, 4);
        host.pulsar(inicio.0, inicio.1);
        host.puntero(destino.0, destino.1);
        let (w, _) = host.shell.dock_buffer_size();
        host.dock
            .buffer
            .render()
            .draw(|datos| {
                let alfa = datos[((8 * w + w / 2) * 4 + 3) as usize];
                assert!(
                    alfa > 150,
                    "el fondo del dock debe seguir visible: alfa {alfa}"
                );
                Ok::<_, std::convert::Infallible>(Vec::new())
            })
            .expect("la inspección del buffer no puede fallar");
    }

    #[test]
    fn el_dock_activa_al_soltar_y_tolera_el_temblor_del_clic() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        let (x, y) = centro_icono_dock(&host, 1);
        assert_eq!(host.pulsar(x, y), None);
        host.puntero(x + 1.0, y + 1.0);
        assert_eq!(
            host.soltar(),
            Some(bookos_shell::Accion::Lanzar("konsole".into()))
        );
        assert!(host.agarrado.is_none());
    }

    #[test]
    fn ordenar_el_dock_no_lanza_y_conserva_el_orden_de_los_vecinos() {
        for escala in [1.0, 1.25, 1.75, 2.0] {
            let mut host =
                super::ShellHost::new(2560, 1600, escala, Some(bookos_shell::Config::default()));
            let inicio = centro_icono_dock(&host, 1);
            let destino = centro_icono_dock(&host, 4);
            assert_eq!(host.pulsar(inicio.0, inicio.1), None);
            host.puntero(destino.0, destino.1);
            assert!(
                host.agarrado.is_some(),
                "el icono sigue al puntero a escala {escala}"
            );
            let orden: Vec<_> = host.shell.dock().items().iter().map(|i| i.exec()).collect();
            assert_eq!(
                orden,
                [
                    "launchpad",
                    "bookos-explorer",
                    "firefox",
                    "bookos-settings",
                    "konsole",
                    "bookos-notepad"
                ]
            );
            let Some(bookos_shell::Accion::GuardarDock(lista)) = host.soltar() else {
                panic!("ordenar guarda el dock sin lanzar una aplicación");
            };
            let config = bookos_shell::Config::desde_texto(
                &format!("dock = {}", lista.join(", ")),
                std::path::Path::new("orden-dock.conf"),
            );
            assert_eq!(config.dock[4].exec, "konsole");
            assert!(host.agarrado.is_none());
            assert!(!host.dock_agarrado());
        }
    }

    #[test]
    fn escape_restaura_el_orden_y_consume_la_suelta_del_arrastre() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        let antes = host.shell.dock().como_configuracion();
        let inicio = centro_icono_dock(&host, 1);
        let destino = centro_icono_dock(&host, 4);
        host.pulsar(inicio.0, inicio.1);
        host.puntero(destino.0, destino.1);
        assert_eq!(host.tecla(bookos_shell::TeclaPulsada::Escape), (true, None));
        assert_eq!(host.shell.dock().como_configuracion(), antes);
        assert!(host.agarrado.is_none());
        host.puntero(-100.0, 500.0);
        assert!(
            host.contiene(-100.0, 500.0, false),
            "retiene el gesto en otro monitor"
        );
        assert_eq!(host.soltar(), None);
        assert!(!host.contiene(-100.0, 500.0, false));
    }

    #[test]
    fn volver_al_origen_tras_arrastrar_no_se_convierte_en_clic() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        let inicio = centro_icono_dock(&host, 1);
        let destino = centro_icono_dock(&host, 3);
        host.pulsar(inicio.0, inicio.1);
        host.puntero(destino.0, destino.1);
        host.puntero(inicio.0, inicio.1);
        assert!(matches!(
            host.soltar(),
            Some(bookos_shell::Accion::GuardarDock(_))
        ));
    }

    #[test]
    fn ordenar_con_el_launchpad_abierto_conserva_la_rejilla() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        host.alternar_launchpad();
        let inicio = centro_icono_dock(&host, 1);
        let destino = centro_icono_dock(&host, 3);
        host.pulsar(inicio.0, inicio.1);
        host.puntero(destino.0, destino.1);
        assert!(matches!(
            host.soltar(),
            Some(bookos_shell::Accion::GuardarDock(_))
        ));
        assert_eq!(host.emergente_nombre(), Some("launchpad"));
        let inicio = centro_icono_dock(&host, 0);
        assert_eq!(host.pulsar(inicio.0, inicio.1), None);
        assert_eq!(host.soltar(), Some(bookos_shell::Accion::Launchpad));
    }

    #[test]
    fn pulsar_una_aplicacion_abierta_conserva_su_accion_de_activacion() {
        let mut host =
            super::ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        host.shell.dock_ventanas(&["konsole".into()]);
        let inicio = centro_icono_dock(&host, 1);
        assert_eq!(host.pulsar(inicio.0, inicio.1), None);
        assert!(
            matches!(host.soltar(), Some(bookos_shell::Accion::Activar { app_id, .. }) if app_id == "konsole")
        );
    }

    use super::*;

    #[test]
    fn ajustes_del_dock_repintan_y_no_recrean_si_no_cambia() {
        let mut host = ShellHost::new(1920, 1080, 1.75, Some(bookos_shell::Config::default()));
        assert!(host.dock_tamano(80));
        assert_eq!(host.shell.dock().tamano_actual(), 80);
        assert!(
            !host.shell.dock_needs_paint(),
            "la superficie nueva no puede quedar vacía"
        );
        assert!(!host.dock_tamano(80));
        assert!(host.dock_tamano(32));
        assert!(!host.shell.dock_needs_paint());
    }

    // añadido en la 0.2.0
    #[test]
    fn reabrir_una_tarjeta_conserva_la_posicion_y_el_impulso() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.abrir_de_widget("volumen");
        host.entrada_sin_ver.set(false);
        let mut muelle = host.muelle_emergente.get();
        muelle.fijar(0.4);
        muelle.soltar(1.0, 2.0);
        host.muelle_emergente.set(muelle);
        host.cerrar_emergente();
        assert_eq!(host.muelle_emergente.get().valor(), 0.4);
        assert_eq!(host.muelle_emergente.get().velocidad(), 2.0);
        host.abrir_de_widget("volumen");
        let muelle = host.muelle_emergente.get();
        assert_eq!(muelle.valor(), 0.4);
        assert_eq!(muelle.velocidad(), 2.0);
        assert_eq!(muelle.destino(), 1.0);
    }

    // añadido en la 0.2.0
    #[test]
    fn agarrar_una_salida_del_launchpad_y_cancelar_continua_la_salida() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.alternar_launchpad();
        let mut muelle = host.muelle_emergente.get();
        muelle.fijar(0.4);
        host.muelle_emergente.set(muelle);
        host.cerrar_emergente();
        host.agarrar_launchpad();
        assert_eq!(host.muelle_emergente.get().valor(), 0.4);
        host.seguir_launchpad(0.1);
        assert_eq!(host.muelle_emergente.get().valor(), 0.5);
        host.soltar_launchpad(crate::gestos::Suelta {
            movimiento: crate::gestos::Movimiento::Launchpad,
            progreso: 0.1,
            velocidad: 2.0,
            cancelada: true,
        });
        assert_eq!(host.muelle_emergente.get().destino(), 0.0);
        assert!(host.cerrando_en.is_some());
    }

    // añadido en la 0.2.0
    #[test]
    fn pellizcar_desde_el_limite_cerrado_invierte_sin_recorrido_muerto() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.agarrar_launchpad();
        host.seguir_launchpad(-0.7);
        assert!(!host.shell.hay_emergente());
        host.seguir_launchpad(-0.6);
        assert_eq!(host.shell.emergente_nombre(), Some("launchpad"));
        assert!((host.muelle_emergente.get().valor() - 0.1).abs() < 0.0001);
    }

    #[test]
    fn la_vista_sigue_el_recorrido_y_se_detiene_con_los_dedos() {
        for escala in [1.0, 1.75, 2.0] {
            let mut host =
                ShellHost::new(1920, 1080, escala, Some(bookos_shell::Config::default()));
            host.seguir_vista(0.05, 0, vec!["Uno".into(), "Dos".into()]);
            let iniciales = host.escritorios_miniaturas();
            assert_eq!(iniciales.len(), 2);
            assert!(iniciales.iter().all(|r| r.loc.y + r.size.h < 0));
            let avance = host.avance_emergente();
            host.animar_emergente();
            assert_eq!(host.avance_emergente(), avance);
            assert_eq!(host.escritorios_miniaturas(), iniciales);

            host.seguir_vista(0.5, 0, vec!["Uno".into(), "Dos".into()]);
            let intermedias = host.escritorios_miniaturas();
            host.seguir_vista(1.0, 0, vec!["Uno".into(), "Dos".into()]);
            let finales = host.escritorios_miniaturas();
            for ((inicial, intermedia), final_) in iniciales.iter().zip(&intermedias).zip(&finales)
            {
                assert!(inicial.loc.y < intermedia.loc.y && intermedia.loc.y < final_.loc.y);
                assert_eq!(inicial.size, final_.size);
                assert_eq!(inicial.loc.x, final_.loc.x);
                let alto = host
                    .emergente
                    .as_ref()
                    .expect("la vista tiene superficie")
                    .logical
                    .h;
                assert!((2 * (intermedia.loc.y - final_.loc.y) + alto).abs() <= 1);
            }
            host.seguir_vista(0.5, 0, vec!["Uno".into(), "Dos".into()]);
            assert_eq!(host.escritorios_miniaturas(), intermedias);
        }
    }

    #[test]
    fn cancelar_la_vista_conserva_las_miniaturas_hasta_terminar_la_salida() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.seguir_vista(0.4, 0, vec!["Uno".into(), "Dos".into()]);
        let miniaturas = host.escritorios_miniaturas();
        host.soltar_vista(crate::gestos::Suelta {
            movimiento: crate::gestos::Movimiento::Vista,
            progreso: 0.4,
            velocidad: 0.0,
            cancelada: true,
        });
        assert!(!host.vista_escritorios_abierta());
        assert!(host.vista_escritorios_visible());
        assert_eq!(host.escritorios_miniaturas(), miniaturas);
        assert_eq!(host.avance_emergente(), 0.4);
        assert_eq!(host.muelle_emergente.get().destino(), 0.0);
        let mut muelle = host.muelle_emergente.get();
        muelle.fijar(0.0);
        host.muelle_emergente.set(muelle);
        host.fin_animacion();
        assert!(!host.vista_escritorios_visible());
        assert!(host.escritorios_miniaturas().is_empty());
    }

    // añadido en la 0.2.0
    #[test]
    fn mover_el_bloqueo_no_rasteriza_sus_capas() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.bloquear("12:30".into(), "miércoles".into(), (1280.0, 800.0));
        super::COSTE_PINTURA.with(|c| c.set(std::time::Duration::ZERO));
        host.animar_bloqueo();
        assert_eq!(
            super::COSTE_PINTURA.with(|c| c.get()),
            std::time::Duration::ZERO
        );
        assert!(host.animando());
    }

    // añadido en la 0.2.0
    #[test]
    fn el_hover_del_dock_no_vuelve_a_pintar_la_barra() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.animar_dock();
        super::COSTE_PINTURA.with(|c| c.set(std::time::Duration::ZERO));
        let (x, y) = centro_icono_dock(&host, 1);
        host.puntero(x, y);
        host.animar_dock();
        assert_eq!(
            super::COSTE_PINTURA.with(|c| c.get()),
            std::time::Duration::ZERO
        );
        assert!(!host.shell.dock_needs_paint());
    }

    #[test]
    fn la_entrada_del_bloqueo_mantiene_vivo_el_reloj_de_frames() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.bloquear(
            "12:30".into(),
            "jueves, 17 de septiembre".into(),
            (1280.0, 800.0),
        );
        assert!(host.shell.bloqueo_animando());
        assert!(host.animando());
    }

    #[test]
    fn una_actualizacion_externa_repinta_la_tarjeta_abierta() {
        let anterior = bookos_system::snapshot();
        let mut datos = anterior.clone();
        datos.audio = serde_json::json!({"output":{"volume":20,"muted":false}});
        bookos_system::seed_test_state(datos.clone());
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.abrir_de_widget("volumen");
        assert!(host.emergente.is_some());
        datos.audio = serde_json::json!({"output":{"volume":70,"muted":true}});
        bookos_system::seed_test_state(datos.clone());
        assert!(host.refresh());
        assert!(
            !host.shell.emergente_needs_paint(),
            "refresh debe subir el dibujo actualizado"
        );
        datos.audio = serde_json::json!({"output":{"volume":35,"muted":false}});
        bookos_system::seed_test_state(datos);
        assert!(host.refresh_emergente());
        assert!(
            !host.shell.emergente_needs_paint(),
            "el refresco de una tarjeta también la pinta"
        );
        bookos_system::seed_test_state(anterior);
    }

    /// El velo de la captura tapa todo menos el recuadro, y el marco va por
    /// dentro de él. Antes lo comprobaba el shell mirando el alfa de su buffer;
    /// ahora el velo son rectángulos del compositor y se comprueba aquí.
    #[test]
    fn el_velo_de_la_captura_deja_el_agujero_del_recuadro() {
        use smithay::backend::renderer::element::Element;

        let mut host = ShellHost::new(2880, 1800, 1.75, Some(bookos_shell::Config::default()));
        host.abrir_captura();
        let velo = host.velo_captura();
        assert_eq!(velo.len(), 1, "sin recuadro el velo es una sola banda");
        assert_eq!(velo[0].geometry(1.0.into()).size, (2880, 1800).into());

        // En lógicos: 100,100 → 500,400, o sea 175,175 → 875,700 en físicos.
        host.captura_pulsar(100.0, 100.0);
        host.captura_puntero(500.0, 400.0);
        let velo = host.velo_captura();
        assert_eq!(velo.len(), 8, "cuatro bandas y cuatro lados del marco");
        let rects: Vec<_> = velo.iter().map(|e| e.geometry(1.0.into())).collect();
        let tapa = |x, y| rects.iter().any(|r| r.contains((x, y)));
        assert!(tapa(10, 10), "la esquina lleva velo");
        assert!(tapa(2870, 1790), "la esquina opuesta también");
        assert!(
            !tapa(500, 400),
            "el centro del recuadro no puede llevar nada"
        );
        assert!(tapa(175, 400), "el marco va en el borde de dentro");
        assert!(!tapa(180, 400), "y no es más ancho que BORDE");
        // Las bandas no se pisan ni dejan huecos: todo lo de fuera tapado una
        // sola vez, o el velo saldría más oscuro en las esquinas.
        let fuera = [(10, 10), (1000, 400), (400, 1000), (100, 400)];
        for (x, y) in fuera {
            let veces = rects.iter().filter(|r| r.contains((x, y))).count();
            assert_eq!(veces, 1, "({x},{y}) tapado {veces} veces");
        }
    }

    /// El buscador **no se mueve** mientras escribes.
    ///
    /// Su alto cambia con cada tecla —cada resultado que entra o sale es media
    /// fila— y antes se recolocaba en cada cambio de tamaño, así que el campo
    /// de texto subía y bajaba mientras tecleabas y el rectángulo de cristal de
    /// detrás saltaba con él. Ahora se coloca por el alto estable del campo. Se
    /// comprueba aquí y no en el shell porque quien coloca es el compositor.
    #[test]
    fn el_buscador_no_salta_al_escribir() {
        use bookos_shell::TeclaPulsada;

        let mut host = ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        host.alternar_buscador();
        assert_eq!(host.shell.emergente_nombre(), Some("buscador"));
        let (_, y0, _, alto0) = host.emergente_rect().expect("el buscador tiene superficie");
        let alto_campo = host
            .shell
            .emergente_geometria()
            .and_then(|(_, _, alto)| alto)
            .expect("el buscador declara su alto estable") as f64;
        let arriba = host.shell.panel_height() as f64;
        let (_, dock_y, _, _) = host.dock_rect();
        let centro_util = arriba + (dock_y - SEPARACION - arriba) / 2.0;
        assert_eq!(
            y0 + alto_campo / 2.0,
            centro_util,
            "el campo de Spotlight no está centrado en el área útil"
        );

        // «sh» encuentra al menos el ejecutable del PATH, que existe en
        // cualquier Linux: la lista aparece y la tarjeta crece.
        for c in "sh".chars() {
            host.tecla(TeclaPulsada::Caracter(c));
        }
        let (_, y1, _, alto1) = host.emergente_rect().expect("sigue abierto");
        assert!(
            alto1 > alto0,
            "la lista no apareció: el test no prueba nada"
        );
        assert_eq!(y1, y0, "la tarjeta se movió al escribir");

        // Y al borrar tampoco vuelve a moverse.
        host.tecla(TeclaPulsada::Retroceso);
        let (_, y2, _, _) = host.emergente_rect().expect("sigue abierto");
        assert_eq!(y2, y0, "la tarjeta se movió al borrar");
    }

    /// El área útil descuenta el dock cuando el dock está a la vista.
    ///
    /// Es la mitad que faltaba de «esquivar ventanas»: el panel reserva su
    /// franja de arriba desde siempre, pero el dock no reservaba nada, así que
    /// una ventana maximizada llegaba hasta el borde de abajo y el dock —que en
    /// modo `Siempre` no se aparta— le tapaba sus últimos píxeles.
    #[test]
    fn el_area_util_no_se_mete_debajo_del_dock() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        assert_eq!(
            host.visibilidad(Barra::Dock),
            Visibilidad::Siempre,
            "el dock arranca a la vista; si no, este test no prueba nada"
        );

        let (_, dock_y, _, dock_h) = host.dock_rect();
        // Lo que el dock quiere que se le reserve por abajo, contando el aire
        // que deja contra el borde de la pantalla.
        let reserva = host.reserva(Barra::Dock);
        assert!(
            reserva >= 800.0 - dock_y,
            "el dock ocupa de y={dock_y} a y={} y solo reserva {reserva}",
            dock_y + dock_h
        );

        // Y en cuanto esquiva deja de reservar: si reservara no habría nada que
        // esquivar, que es lo mismo que ya hace el panel.
        host.alternar_visibilidad(Barra::Dock);
        assert_eq!(host.visibilidad(Barra::Dock), Visibilidad::Esquivando);
        assert!(host.dock_barra.escondida, "el dock no inició su ocultación");
        assert_eq!(host.reserva(Barra::Dock), 0.0, "esquivando no reserva");
    }

    // añadido en la 0.2.0 -- los puntos de página quedaban sobre el hueco del dock escondido.
    #[test]
    fn el_dock_escondido_no_le_quita_sitio_al_launchpad() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        assert!(
            host.alto_launchpad() < 800.0,
            "a la vista, el dock se reserva"
        );
        host.dock_barra.modo = Visibilidad::Esquivando;
        host.dock_barra.escondida = true;
        assert_eq!(host.alto_launchpad(), 800.0);
        // Pedido con el ratón vuelve a estar delante y vuelve a reservarse.
        host.dock_barra.pedida = true;
        assert!(host.alto_launchpad() < 800.0);
    }

    /// Con el dock apartado su franja no es de nadie: el clic tiene que llegar
    /// a la ventana de debajo, no a un icono que no se ve.
    #[test]
    fn el_dock_escondido_no_recibe_clics() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        let (dx, dy, dw, dh) = host.dock_rect();
        let (x, y) = (dx + dw / 2.0, dy + dh / 2.0);
        assert!(
            host.contiene(x, y, false),
            "a la vista, el dock es del shell"
        );

        // Escondido del todo, sin esperar a la animación.
        host.dock_barra.escondida = true;
        host.dock_barra.desde_fraccion = 1.0;
        assert!(!host.a_la_vista(Barra::Dock));
        assert!(
            host.a_la_vista(Barra::Panel),
            "el caso del fallo: panel sí, dock no"
        );
        assert!(
            !host.contiene(x, y, false),
            "la franja del dock fantasma se tragaba el clic"
        );
        assert_eq!(host.pulsar(x, y), None);
    }

    /// Cerrar una tarjeta pulsando fuera devuelve el panel a su sitio sin
    /// esperar a que se mueva el ratón: se abrió con el cursor arriba, pero
    /// para pulsar fuera se bajó, y eso es lo que manda al cerrar.
    #[test]
    fn cerrar_la_tarjeta_esconde_el_panel_sin_mover_el_raton() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.alternar_visibilidad(Barra::Panel);
        assert_eq!(host.visibilidad(Barra::Panel), Visibilidad::Esquivando);
        host.estorbada(Barra::Panel, true);

        // El cursor arriba para pulsar el widget: el panel vuelve.
        host.puntero(640.0, 2.0);
        host.reclamada(Barra::Panel, (640.0, 2.0));
        host.abrir_acerca();
        assert_eq!(host.panel_barra.destino(), 0.0);

        // Se baja a pulsar fuera, con la ventana aún debajo.
        host.puntero(640.0, 400.0);
        host.reclamada(Barra::Panel, (640.0, 400.0));
        host.estorbada(Barra::Panel, true);
        assert_eq!(host.panel_barra.destino(), 0.0, "abierta, la retiene");

        host.cerrar_emergente();
        assert_eq!(host.panel_barra.destino(), 1.0, "cerrada, se aparta");
    }

    /// La tarjeta de la isla cuelga por debajo del panel: bajar a sus
    /// controles no puede apartarlo, y recogerla pulsando fuera sí.
    #[test]
    fn zenith_sigue_en_pantalla_con_el_panel_oculto_y_el_resto_no() {
        use bookos_shell::actividad::{Clase, Estado, Fase};
        let con_clase = |clase| {
            let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
            host.publicar_actividad(Estado {
                app_id: "com.bookos.test".into(),
                clase,
                activo: true,
                pausado: false,
                titulo: "Título".into(),
                subtitulo: String::new(),
                posicion_ms: 0,
                duracion_ms: 0,
                restante_ms: 0,
                volumen: 50,
                nivel: 0.0,
                aleatorio: false,
                repetir: false,
                portada: None,
                cola: Vec::new(),
                metricas: Vec::new(),
            });
            host.panel_barra.escondida = true;
            host.panel_barra.desde_fraccion = 1.0;
            assert!(!host.a_la_vista(Barra::Panel));
            host.actividad_rect().is_some()
        };
        assert!(con_clase(Clase::Asistente(Fase::Escuchando)));
        assert!(con_clase(Clase::Asistente(Fase::Respuesta)));
        assert!(!con_clase(Clase::Player));
    }

    #[test]
    fn la_isla_desplegada_retiene_el_panel() {
        use bookos_shell::actividad::{Clase, Estado};
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.alternar_visibilidad(Barra::Panel);
        host.estorbada(Barra::Panel, true);
        host.publicar_actividad(Estado {
            app_id: "com.bookos.test".into(),
            clase: Clase::Player,
            activo: true,
            pausado: false,
            titulo: "Pista".into(),
            subtitulo: "Artista".into(),
            posicion_ms: 0,
            duracion_ms: 60_000,
            restante_ms: 0,
            volumen: 50,
            nivel: 0.0,
            aleatorio: false,
            repetir: false,
            portada: None,
            cola: Vec::new(),
            metricas: Vec::new(),
        });

        // El cursor arriba para pulsar la isla: el panel vuelve y se despliega.
        host.puntero(640.0, 2.0);
        host.reclamada(Barra::Panel, (640.0, 2.0));
        assert!(host.abrir_actividad_previsualizacion("com.bookos.test"));

        // Se baja hacia los controles, con la ventana aún debajo.
        host.puntero(640.0, 200.0);
        host.reclamada(Barra::Panel, (640.0, 200.0));
        host.estorbada(Barra::Panel, true);
        assert_eq!(host.panel_barra.destino(), 0.0, "desplegada, lo retiene");

        assert!(host.recoger_actividad_fuera(640.0, 700.0));
        assert_eq!(host.panel_barra.destino(), 1.0, "recogida, se aparta");
    }

    #[test]
    fn cambiar_el_panel_cierra_su_emergente() {
        let mut host = ShellHost::new(1280, 800, 1.0, Some(bookos_shell::Config::default()));
        host.abrir_acerca();
        assert!(
            host.shell.hay_emergente(),
            "no se abrió la tarjeta de prueba"
        );
        host.alternar_visibilidad(Barra::Panel);
        assert!(
            !host.shell.hay_emergente(),
            "la tarjeta quedó huérfana del panel"
        );
    }

    /// Una tarjeta que cuelga del panel no toca el borde de la pantalla, y la
    /// franja de escritorios sigue pegada a él.
    ///
    /// El calendario cuelga del reloj, que está en la esquina superior derecha,
    /// así que su sitio natural cae fuera de la pantalla. Antes se recortaba a
    /// cero y quedaba pegado al borde exacto: su sombra cae fuera del buffer,
    /// desaparecía justo por ese lado y la tarjeta se leía como cortada. Pero
    /// el arreglo no puede ser dar margen a todo: la franja de escritorios se
    /// ancla al borde de arriba a propósito y ahí 8 px la descolocan.
    #[test]
    fn el_calendario_no_toca_el_borde_y_la_franja_si() {
        let mut host = ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));

        host.abrir_de_widget("reloj");
        assert_eq!(host.shell.emergente_nombre(), Some("calendario"));
        let (x, y, w, h) = host
            .emergente_rect()
            .expect("el calendario tiene superficie");
        assert!(x >= SEPARACION, "pegado al borde izquierdo: x={x}");
        assert!(y >= SEPARACION, "pegado al borde de arriba: y={y}");
        assert!(
            x + w <= 1920.0 - SEPARACION,
            "pegado al borde derecho: x+w={} de 1920",
            x + w
        );
        assert!(
            y + h <= 1080.0 - SEPARACION,
            "pegado al borde de abajo: y+h={} de 1080",
            y + h
        );

        host.cerrar_emergente();
        host.alternar_vista_escritorios(0, vec!["Uno".into(), "Dos".into()]);
        assert_eq!(host.shell.emergente_nombre(), Some("escritorios"));
        let (x, y, _, _) = host.emergente_rect().expect("la franja tiene superficie");
        assert_eq!((x, y), (0.0, 0.0), "la franja se ha descolocado del borde");
    }

    /// El shell vive solo en la pantalla principal y no puede quedarse con los
    /// clics de las demás.
    ///
    /// Sin esto, con una pantalla colocada **a la izquierda** —que en el
    /// escritorio normalizado tiene `x` negativa— pasaban dos cosas: la franja
    /// de arriba caía dentro del panel de la principal y se tragaba los clics,
    /// y con cualquier emergente abierta el shell reclamaba la pantalla
    /// secundaria **entera**, así que el ratón dejaba de funcionar ahí hasta
    /// cerrarla. Es un caso que no se ve sin dos monitores enchufados, de ahí
    /// el test.
    #[test]
    fn el_shell_no_reclama_los_clics_de_otra_pantalla() {
        // 1920×1080 a escala 1: el área lógica de la principal es (0,0)-(1920,1080).
        let mut host = ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        assert_eq!(host.area_principal(), (1920.0, 1080.0));

        // Dentro de la principal, la franja del panel es suya.
        assert!(host.a_la_vista(Barra::Panel), "el panel arranca a la vista");
        assert!(
            host.contiene(960.0, 4.0, false),
            "la franja del panel es del shell"
        );

        // La misma altura, pero en la pantalla de la izquierda.
        assert!(
            !host.contiene(-400.0, 4.0, false),
            "x negativa es otro monitor: el clic es del cliente que haya ahí"
        );
        // Y a la derecha, pasado el borde.
        assert!(
            !host.contiene(2400.0, 4.0, false),
            "pasado el ancho tampoco es suyo"
        );

        // Con una emergente abierta el shell se queda con todo lo de la
        // principal —pulsar fuera la cierra— pero **solo** con lo de la
        // principal.
        host.abrir_acerca();
        assert!(host.shell.hay_emergente(), "no se abrió la emergente");
        assert!(
            host.contiene(960.0, 600.0, false),
            "con una emergente, el clic en la principal la cierra"
        );
        assert!(
            !host.contiene(-400.0, 600.0, false),
            "una emergente en la principal no puede tragarse el ratón del monitor de al lado"
        );

        // Y lo mismo para el escritorio: el hueco vacío de otra pantalla no
        // arranca una banda elástica aquí.
        assert!(host.escritorio_alcanza(960.0, 600.0));
        assert!(!host.escritorio_alcanza(-400.0, 600.0));
    }
}

#[cfg(test)]
mod pruebas_transicion {
    use super::*;

    #[test]
    fn el_detalle_crece_desde_el_boton_y_se_recoge_en_el_mismo_sitio() {
        let tamano = (352.0, 420.0);
        let origen = (180.0, 80.0, 156.0, 52.0);
        for progreso in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let entrada = ShellHost::geometria_detalle(tamano, origen, progreso, false);
            let salida = ShellHost::geometria_detalle(tamano, origen, 1.0 - progreso, true);
            assert_eq!(entrada, salida, "la salida no deshace la entrada");
        }
        let (_, zoom, x, y) = ShellHost::geometria_detalle(tamano, origen, 0.0, false);
        assert_eq!(tamano.0 / 2.0 + x, origen.0 + origen.2 / 2.0);
        assert_eq!(tamano.1 / 2.0 + y, origen.1 + origen.3 / 2.0);
        assert!((tamano.1 * zoom - origen.3).abs() < 0.001);
        assert_eq!(
            ShellHost::geometria_detalle(tamano, origen, 1.0, false),
            (1.0, 1.0, 0.0, 0.0)
        );
    }

    /// Entrar en una carpeta guarda la vista de antes y deja al compositor
    /// dibujando mientras dure la transición.
    ///
    /// Es lo único que se puede comprobar sin una GPU: que la superficie vieja
    /// se aparta en vez de repintarse encima —que era el fallo— y que
    /// `animando()` sigue diciendo que sí, que es lo que hace que lleguen los
    /// fotogramas del cruce.
    #[test]
    fn abrir_una_carpeta_guarda_la_vista_anterior() {
        let dir = std::env::temp_dir().join("bookos-test-transicion");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bookos")).unwrap();
        std::fs::write(
            dir.join("bookos/launchpad.conf"),
            "Utilidades = konsole, kate, dolphin\n",
        )
        .unwrap();
        // SAFETY: este test no comparte la variable con nadie más; se restaura
        // al final.
        let antes = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

        let mut host = ShellHost::new(1920, 1080, 1.0, None);
        host.alternar_launchpad();
        assert!(host.emergente.is_some(), "el launchpad tiene superficie");
        assert!(host.vista_anterior.is_none());

        // La carpeta va la primera de la rejilla. Se pulsa por el centro de su
        // celda, en coordenadas de pantalla.
        let (x, y) = host.shell.launchpad_celda(0).expect("hay primera celda");
        let (ox, oy, _, _) = host.emergente_rect().expect("y está colocada");
        host.pulsar(ox + x as f64, oy + y as f64);
        host.soltar();

        assert!(host.shell.realce_delante(), "se entró en la carpeta");
        assert!(
            host.vista_anterior.is_some(),
            "la rejilla de antes tiene que quedarse para el cruce"
        );
        assert!(host.animando(), "y el compositor tiene que seguir pintando");

        // Y salir hace lo propio, con su transición de cierre.
        host.vista_anterior = None;
        host.tecla(bookos_shell::TeclaPulsada::Escape);
        assert!(!host.shell.realce_delante(), "Esc sale de la carpeta");
        assert!(
            host.vista_anterior.is_some(),
            "el panel de antes también tiene que quedarse para el cruce"
        );

        match antes {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
    }
}
