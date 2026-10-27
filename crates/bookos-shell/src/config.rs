//! Qué widgets lleva el panel y en qué orden.
//!
//! El fichero es `~/.config/bookos/panel.conf` y tiene esta pinta:
//!
//! ```text
//! # Los widgets del panel, por zona.
//! centro = reloj
//! derecha = escritorios, red, brillo, bateria
//! escala = 1.75
//! tema = oscuro
//! acento = azul
//! avatar = ~/Imágenes/yo.png
//! launchpad_dock = si
//! bloqueo_animaciones = si
//! bloqueo_fecha = si
//! bloqueo_medios = si
//! bloqueo_nombre = si
//! bloqueo_energia = si
//! bloqueo_reloj_y = 0.08
//! bloqueo_acceso_y = 0.36
//! bloqueo_medios_y = 0.68
//! bloqueo_reloj_tamano = 120
//! bloqueo_avatar_tamano = 112
//! bloqueo_reloj_fuente = moderna
//! bloqueo_reloj_grosor = 400
//! atenuacion_inactividad = 300
//! bloqueo_inactividad = 900
//! salvapantallas_inactividad = 1200
//! salvapantallas_estilo = estela
//! apagado_inactividad = 1800
//! suspension_inactividad = 0
//! actividades = si
//! actividades_animaciones = si
//! alto_contraste = no
//! brillo_automatico_pantalla = no
//! brillo_automatico_teclado = no
//! luz_nocturna = programada
//! luz_nocturna_temperatura = 4000
//! luz_nocturna_desde = 22:00
//! luz_nocturna_hasta = 07:00
//! temporizador_siempre_visible = no
//! sonido_volumen = si
//! sonidos_interfaz = si
//! volumen_paso = 5
//! volumen_paso_mantener = 2
//! brillo_paso = 5
//! brillo_paso_mantener = 2
//! genio = si
//! panel_esquiva = no
//! dock_esquiva = no
//! iconos_escritorio = si
//! escritorios = 2
//! nombres_escritorios = Escritorio 1, Escritorio 2
//! dock = konsole:Terminal:utilities-terminal, bookos-explorer:Archivos:bookos-explorer, firefox:Navegador:firefox, bookos-settings:Ajustes:bookos-settings
//!
//! # Cursor y entrada. Las velocidades van en la escala de libinput: [-1, 1].
//! # Uno para cada tema: el cambio de claro a oscuro se lleva el fondo con él.
//! fondo_claro = /usr/share/wallpapers/BookOS/Light/blue.png
//! fondo_oscuro = /usr/share/wallpapers/BookOS/Dark/blue_dark.png
//! cursor = 24
//! velocidad_touchpad = 0.3
//! toque_para_clic = si
//! scroll_natural = si
//! pantalla_tactil = si
//! ```
//!
//! **Por qué no TOML.** Leer dos listas de nombres no justifica meter `toml`,
//! que arrastra `serde` entero en un crate que hoy no tiene ninguna
//! dependencia de serialización. El formato de arriba se analiza en veinte
//! líneas y no puede fallar de formas interesantes.
//!
//! Si el fichero no existe se usan los valores de [`Config::default`], así que
//! el panel se sigue pintando en el primer frame sin tocar el disco. Y si
//! existe pero tiene una errata, la línea mala se ignora y se avisa: quedarse
//! sin panel por una coma es peor que quedarse sin un widget.

use std::collections::HashMap;
use std::path::PathBuf;

/// Cuánto adorno se dibuja.
///
/// `Reducidos` quita el desenfoque del panel y del dock y las animaciones de
/// ventana —abrir, minimizar, encajar, mover—. No quita las del propio shell
/// (emergentes, launchpad, cambio de escritorio): esas duran menos de 300 ms
/// sobre buffers pequeños y no son lo que se come los fotogramas.
///
/// La batería baja lo enciende sola sin tocar esta clave: el usuario elige lo
/// que quiere **cuando hay energía para ello**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Efectos {
    #[default]
    Completos,
    Reducidos,
}

/// Qué luces siguen al sensor de luz ambiente.
///
/// Dos interruptores y no un enum de cuatro casos: en la tarjeta de brillo
/// cada fila tiene su botón y se encienden por separado. Los dos apagados de
/// serie, porque un brillo que cambia solo sin haberlo pedido parece un fallo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BrilloAutomatico {
    pub pantalla: bool,
    pub teclado: bool,
}

/// Cuándo se tiñe la pantalla de cálido.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModoLuzNocturna {
    #[default]
    Apagada,
    Encendida,
    /// De `desde` a `hasta`, cada día.
    Programada,
}

/// La luz nocturna: el compositor tiñe la salida bajando la curva de gamma de
/// los canales verde y azul (ver `bookos-comp/src/luz_nocturna.rs`).
///
/// La temperatura va en kelvin, la unidad en que se describe el color de una
/// luz: 6500 es el blanco del panel y no tiñe nada, por debajo es cálida y por
/// encima fría. Se deja subir hasta [`LuzNocturna::MAXIMA`] porque el
/// deslizador de Ajustes va de cálida a fría y alguien puede querer el blanco
/// algo más azulado de día.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuzNocturna {
    pub modo: ModoLuzNocturna,
    pub temperatura: u32,
    pub desde: crate::tema::HoraDelDia,
    pub hasta: crate::tema::HoraDelDia,
}

impl LuzNocturna {
    pub const NEUTRA: u32 = 6500;
    /// 2500 K es una vela larga: más abajo el azul desaparece del todo y los
    /// colores de la interfaz dejan de distinguirse —acento, rojo y amarillo
    /// salen naranjas—.
    pub const MINIMA: u32 = 2500;
    pub const MAXIMA: u32 = 8000;

    /// ¿Toca teñir ahora, según el modo y el reloj?
    pub fn toca(&self, ahora: crate::tema::HoraDelDia) -> bool {
        match self.modo {
            ModoLuzNocturna::Apagada => false,
            ModoLuzNocturna::Encendida => true,
            ModoLuzNocturna::Programada => crate::tema::en_tramo(self.desde, self.hasta, ahora),
        }
    }

    /// Cuántos minutos faltan para que el horario la encienda o la apague.
    /// `None` fuera del modo programado: no hay nada que esperar.
    pub fn minutos_al_cambio(&self, ahora: crate::tema::HoraDelDia) -> Option<u32> {
        match self.modo {
            ModoLuzNocturna::Programada => {
                crate::tema::minutos_al_borde(self.desde, self.hasta, ahora)
            }
            ModoLuzNocturna::Apagada | ModoLuzNocturna::Encendida => None,
        }
    }
}

impl Default for LuzNocturna {
    fn default() -> Self {
        Self {
            modo: ModoLuzNocturna::Apagada,
            // Lo bastante cálida para notarse sin que el blanco parezca papel
            // viejo: es lo que dejan de serie GNOME y KDE (4000 y 4500).
            temperatura: 4000,
            desde: (22, 0),
            hasta: (7, 0),
        }
    }
}

/// Si la luz nocturna está tiñendo **ahora**. La escribe el compositor cada vez
/// que la aplica y la leen la baldosa y la tarjeta de brillo, que no reciben la
/// configuración: con el modo programado y un cambio a mano, lo que dice el
/// fichero y lo que se ve no tienen por qué coincidir.
static LUZ_NOCTURNA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn luz_nocturna_encendida() -> bool {
    LUZ_NOCTURNA.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn marcar_luz_nocturna(encendida: bool) {
    LUZ_NOCTURNA.store(encendida, std::sync::atomic::Ordering::Relaxed);
}

/// Cuánto mueven las teclas de volumen y brillo, en %: al pulsar y en cada
/// repetición al mantener.
///
/// Al mantener va más fino que al pulsar porque se repite unas diez veces por
/// segundo: con 5 % por repetición el volumen cruzaba de 0 a 100 en dos
/// segundos y no daba tiempo a soltar donde se quería.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pasos {
    pub volumen: u8,
    pub volumen_mantener: u8,
    pub brillo: u8,
    pub brillo_mantener: u8,
}

impl Pasos {
    pub const MINIMO: u8 = 1;
    /// Al pulsar: más de 10 % por pulsación pasa de largo en dos o tres toques.
    pub const MAXIMO_PULSAR: u8 = 10;
    /// Al mantener. Más alto que al pulsar a propósito: es el techo que se
    /// puede elegir, no el valor por defecto (2).
    pub const MAXIMO_MANTENER: u8 = 30;

    /// El techo de una clave de `panel.conf`; `None` si no es un paso.
    pub fn maximo_de(clave: &str) -> Option<u8> {
        match clave {
            "volumen_paso" | "brillo_paso" => Some(Self::MAXIMO_PULSAR),
            "volumen_paso_mantener" | "brillo_paso_mantener" => Some(Self::MAXIMO_MANTENER),
            _ => None,
        }
    }
}

impl Default for Pasos {
    /// 5 % al pulsar es el paso de KDE: con 10 se pasa de largo y con 2 hay
    /// que machacar la tecla.
    fn default() -> Self {
        Self {
            volumen: 5,
            volumen_mantener: 2,
            brillo: 5,
            brillo_mantener: 2,
        }
    }
}

pub struct Config {
    pub dock_tamano: u32,
    pub bloqueo_huella: bool,
    pub centro: Option<String>,
    pub derecha: Vec<String>,
    pub dock: Vec<Lanzador>,
    /// Escala de la pantalla. `None` = la que deduzca el compositor del tamaño
    /// físico del panel.
    ///
    /// Vive aquí aunque el shell no la use porque es lo que el usuario entiende
    /// por "la configuración del escritorio", y tener dos ficheros —uno para el
    /// panel y otro para la pantalla— es la clase de reparto que solo tiene
    /// sentido para quien escribió el código.
    pub escala: Option<f64>,
    /// Tamaño **lógico** del cursor. El tema elige luego qué imagen de las que
    /// trae se acerca más a ese tamaño por la escala de la pantalla.
    pub cursor: u32,
    /// ¿Se ve el dock con el launchpad abierto?
    ///
    /// Es lo que hace macOS, y no es cosmético: es lo que convierte el
    /// launchpad en el sitio de donde se sacan los iconos para anclarlos.
    /// Quien prefiera la rejilla sola lo apaga y pierde el arrastre, no el
    /// launchpad.
    pub launchpad_dock: bool,
    pub entrada: Entrada,
    /// Imagen del fondo del escritorio, la misma con los dos temas.
    /// `None` = la que se encuentre.
    ///
    /// Para tener una con cada tema están `fondo_claro` y `fondo_oscuro`, que
    /// mandan sobre esta: los fondos de BookOS vienen emparejados —`blue.png` y
    /// `blue_dark.png`, y lo mismo para las otras tres familias— y el cambio de
    /// tema tiene que llevarse el fondo con él o el escritorio se queda claro
    /// con una imagen oscura detrás.
    pub fondo: Option<String>,
    /// El fondo con el tema claro. Si está, manda sobre `fondo`.
    pub fondo_claro: Option<String>,
    /// El fondo con el tema oscuro. Si está, manda sobre `fondo`.
    pub fondo_oscuro: Option<String>,
    /// Distribución de teclado (`es`, `us`, `fr`…). `None` = la del sistema.
    pub teclado: Option<String>,
    /// Milisegundos con la tecla pulsada antes de que empiece a repetirse.
    pub teclado_retardo: u32,
    /// Repeticiones por segundo; 0 apaga la repetición, que es lo que
    /// significa una tasa 0 en `wl_keyboard.repeat_info`.
    pub teclado_velocidad: u32,
    /// Cuánto mueven las teclas de volumen y brillo, en %.
    pub pasos: Pasos,
    /// Cuántos escritorios virtuales hay.
    ///
    /// Dos de serie y un máximo de cinco, que es lo que cabe en la vista
    /// general sin convertir las previsualizaciones en sellos.
    pub escritorios: usize,
    /// Nombres editables de los escritorios, en el mismo orden.
    pub nombres_escritorios: Vec<String>,
    /// Claro u oscuro. Lo aplica quien crea el shell, porque el tema es del
    /// proceso entero y no de una superficie.
    /// El tema que toca **ahora mismo**, ya resuelto.
    ///
    /// Con `modo_tema` en automático esto cambia solo a lo largo del día; lo
    /// deriva `cargar` y nadie más, para que no haya dos verdades.
    pub tema: crate::tema::Tema,
    /// Lo que el usuario eligió: claro, oscuro o que siga la hora.
    pub modo_tema: crate::tema::ModoTema,
    /// A qué hora empieza el tramo claro, con `modo_tema` en automático.
    pub tema_claro_desde: crate::tema::HoraDelDia,
    /// Y a qué hora empieza el oscuro. Puede ser anterior al claro: entonces el
    /// tramo claro cruza la medianoche.
    ///
    /// Son dos horas y no el amanecer de verdad porque calcular el amanecer
    /// pide una posición, y sacarla del sistema significa geoclue —un servicio
    /// con su permiso y su agente— o pedirle al usuario que escriba latitud y
    /// longitud en un fichero. Dos horas se entienden solas y se cambian en un
    /// segundo. Si algún día hay posición, esto se calcula desde ella sin que
    /// cambie nada más: el resto del código ya solo habla de estas dos horas.
    pub tema_oscuro_desde: crate::tema::HoraDelDia,
    /// El color de acento, de la tabla cerrada de [`crate::tema::Acento`].
    /// Efectos visuales: completos o reducidos. Ver [`Efectos`].
    pub efectos: Efectos,
    /// Paleta con bordes y separadores reforzados para mejorar la lectura.
    pub alto_contraste: bool,
    pub acento: crate::tema::Acento,
    /// Foto de perfil para el bloqueo. `None` = la del sistema (`~/.face` o
    /// AccountsService), y si tampoco hay, las iniciales.
    pub avatar: Option<String>,
    /// Composición visual del bloqueo. Vive en la configuración compartida
    /// para que BookOS Settings pueda editarla sin conocer el código de iced.
    pub bloqueo: Bloqueo,
    /// Segundos sin entrada antes de bloquear automáticamente. Cero desactiva
    /// el bloqueo automático; por defecto son quince minutos.
    pub bloqueo_inactividad: u64,
    /// Segundos sin entrada antes de bajar el brillo. Cero la desactiva.
    pub atenuacion_inactividad: u64,
    /// Segundos sin entrada antes del salvapantallas, que sale también sobre
    /// el bloqueo: por defecto, cinco minutos después de él. Cero lo desactiva.
    pub salvapantallas_inactividad: u64,
    pub salvapantallas_estilo: EstiloSalvapantallas,
    /// Segundos sin entrada antes de apagar la pantalla. Cero lo desactiva.
    /// La suspensión sigue contando aparte desde el bloqueo.
    pub apagado_inactividad: u64,
    /// Segundos que permanece bloqueada la sesión antes de suspender. Cero la
    /// desactiva: es una política optativa y nunca debe sorprender al usuario.
    pub suspension_inactividad: u64,
    /// Isla de tareas vivas; estas claves quedan preparadas para BookOS
    /// Settings y permiten desactivar movimiento sin apagar la función.
    pub actividades: Actividades,
    pub brillo_automatico: BrilloAutomatico,
    pub luz_nocturna: LuzNocturna,
    /// El chasquido al subir o bajar el volumen con las teclas.
    pub sonido_volumen: bool,
    /// Los sonidos de bloquear, desbloquear, capturar e iniciar sesión.
    pub sonidos_interfaz: bool,
    /// El «magic lamp» al minimizar. Apagado, la ventana se encoge sin deformarse.
    pub genio: bool,
    /// Si el panel y el dock se apartan de las ventanas (Meta+Alt+B y
    /// Meta+Alt+D). Se guardan al cambiar para que sobrevivan al reinicio.
    pub panel_esquiva: bool,
    pub dock_esquiva: bool,
    /// Si se ven los iconos de la carpeta Escritorio sobre el fondo.
    pub iconos_escritorio: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actividades {
    pub habilitadas: bool,
    pub animaciones: bool,
    pub temporizador_siempre: bool,
}

impl Default for Actividades {
    fn default() -> Self {
        Self {
            habilitadas: true,
            animaciones: true,
            temporizador_siempre: false,
        }
    }
}

/// Opciones de la pantalla de bloqueo.
///
/// Las posiciones son fracciones del alto lógico de la salida. De ese modo un
/// valor guardado sirve igual en 1920×1080, 2880×1800 y con escala fraccional:
/// la densidad cambia los píxeles físicos, no la composición que ve el usuario.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bloqueo {
    pub animaciones: bool,
    pub fecha: bool,
    pub medios: bool,
    /// El nombre del usuario bajo el avatar.
    pub nombre: bool,
    /// El botón de apagado de la esquina. Sin él, apagar pide desbloquear antes.
    pub energia: bool,
    pub reloj_y: f32,
    pub acceso_y: f32,
    pub medios_y: f32,
    pub reloj_tamano: f32,
    pub avatar_tamano: f32,
    /// La familia de la hora y de la fecha.
    pub reloj_fuente: FuenteReloj,
    /// El grosor de la hora, de 100 a 900 en pasos de 100: los nueve que
    /// distingue iced (`Weight::Thin`…`Black`). La fecha no lo sigue: va en
    /// Medium, como en macOS, para que una hora Thin no la vuelva ilegible.
    pub reloj_grosor: u16,
}

/// Los estilos de reloj que se ofrecen, como los de la pantalla de bloqueo de
/// macOS. Lista cerrada y no «cualquier fuente del sistema»: casi todas las
/// instaladas traen dos o tres grosores y el control de grosor no haría nada, y
/// Qué pinta el salvapantallas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EstiloSalvapantallas {
    /// Hileras curvas de puntos luminosos que vagan y cambian de color, a la
    /// manera de las teles de 2023.
    #[default]
    Estela,
    /// BookOS rebotando contra los bordes, como el logo de los DVD.
    Rebote,
}

impl EstiloSalvapantallas {
    pub fn nombre(self) -> &'static str {
        match self {
            Self::Estela => "estela",
            Self::Rebote => "rebote",
        }
    }

    pub fn desde_nombre(nombre: &str) -> Option<Self> {
        [Self::Estela, Self::Rebote]
            .into_iter()
            .find(|e| e.nombre() == nombre)
    }

    pub fn otro(self) -> Self {
        match self {
            Self::Estela => Self::Rebote,
            Self::Rebote => Self::Estela,
        }
    }
}

/// `Family::Name` pide un `&'static str`. Todas son variables con eje `wght`
/// completo, y cosmic-text aplica el grosor pedido a ese eje.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FuenteReloj {
    /// Noto Sans: la única que la ISO instala seguro (google-noto-sans-fonts),
    /// por eso es la de serie.
    #[default]
    Moderna,
    Neutra,
    Redondeada,
    Clasica,
    Display,
    Mono,
}

impl FuenteReloj {
    pub const TODAS: [Self; 6] = [
        Self::Moderna,
        Self::Neutra,
        Self::Redondeada,
        Self::Clasica,
        Self::Display,
        Self::Mono,
    ];

    /// Cómo se escribe en `panel.conf` y en `GetConfig`.
    pub fn nombre(self) -> &'static str {
        match self {
            Self::Moderna => "moderna",
            Self::Neutra => "neutra",
            Self::Redondeada => "redondeada",
            Self::Clasica => "clasica",
            Self::Display => "display",
            Self::Mono => "mono",
        }
    }

    pub fn desde_nombre(nombre: &str) -> Option<Self> {
        Self::TODAS.into_iter().find(|f| f.nombre() == nombre)
    }

    /// La familia tal como la conoce fontdb. Sin alias de fontconfig: fontdb
    /// no los lee, así que tiene que ser el nombre exacto de la fuente.
    pub fn familia(self) -> &'static str {
        match self {
            Self::Moderna => "Noto Sans",
            Self::Neutra => "Adwaita Sans",
            // La de la marca: la instala bookos-branding desde BookOS-Fonts.
            Self::Redondeada => "Nunito",
            Self::Clasica => "Noto Serif",
            Self::Display => "Red Hat Display",
            Self::Mono => "Noto Sans Mono",
        }
    }
}

impl Default for Bloqueo {
    fn default() -> Self {
        Self {
            animaciones: true,
            fecha: true,
            medios: true,
            nombre: true,
            energia: true,
            // La referencia: reloj en el primer tercio, identidad en el centro
            // y la tarjeta de lo que suena por debajo del acceso.
            reloj_y: 0.08,
            acceso_y: 0.36,
            medios_y: 0.68,
            reloj_tamano: 144.0,
            avatar_tamano: 132.0,
            reloj_fuente: FuenteReloj::Moderna,
            reloj_grosor: 400,
        }
    }
}

/// Lo que se le pide a libinput sobre los dispositivos de entrada.
///
/// Son los cuatro ajustes que se tocan de verdad en un portátil. El resto de
/// libinput se queda en sus valores por defecto: exponerlo entero sería un
/// panel de control, no un fichero de configuración.
pub struct Entrada {
    /// Velocidad del touchpad, en la escala de libinput: [-1, 1], 0 = normal.
    pub velocidad_touchpad: f64,
    /// Velocidad del ratón y del trackpoint, misma escala.
    pub velocidad_raton: f64,
    pub toque_para_clic: bool,
    pub scroll_natural: bool,
    /// La pantalla táctil. Apagada, libinput no manda ni un toque: la palma
    /// sobre la pantalla de un convertible deja de pulsar cosas.
    pub pantalla_tactil: bool,
}

impl Default for Entrada {
    fn default() -> Self {
        Self {
            // 0,3 y no 0: con la velocidad de serie de libinput hay que dar
            // tres pasadas al touchpad para cruzar una pantalla de 2880 px.
            // Es el mismo valor que este escritorio tiene puesto en KDE
            // (`PointerAcceleration=0.300` en `kcminputrc`), o sea el que ya
            // estaba calibrado a mano contra esta máquina.
            velocidad_touchpad: 0.3,
            // El ratón se queda en el de serie: tiene su propia resolución y
            // acelerarlo por defecto lo vuelve nervioso.
            velocidad_raton: 0.0,
            toque_para_clic: true,
            scroll_natural: true,
            pantalla_tactil: true,
        }
    }
}

/// Cuántos escritorios virtuales caben como mucho.
///
/// Cinco es lo que entra en la vista general sin convertir las
/// previsualizaciones en sellos, y es también hasta dónde llegan los iconos
/// numerados del aviso. El compositor tiene el mismo tope y por eso este valor
/// no puede subir aquí sin subir allí.
pub const MAXIMO_ESCRITORIOS: usize = 5;

/// El `exec` reservado que abre el launchpad en vez de lanzar un programa.
///
/// Es una palabra y no un binario porque el launchpad no es un programa: vive
/// dentro del shell. Quien quiera quitarlo del dock solo tiene que dejarlo
/// fuera de la lista `dock` de su configuración.
pub const LAUNCHPAD: &str = "launchpad";

/// Un lanzador del dock, tal como sale de la configuración.
pub struct Lanzador {
    pub exec: String,
    pub etiqueta: String,
    pub icono: String,
    /// El `app_id` de Wayland con el que se reconoce su ventana.
    pub app_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // El centro va vacío y el reloj el último de la derecha, como la
            // referencia del panel y como el propio plasmoide. La capa central
            // sigue existiendo para quien quiera poner ahí algo —`centro =
            // reloj` la recupera—, pero ya no es lo de serie.
            centro: None,
            // El orden del diseño, de izquierda a derecha: lo que cambia solo
            // —batería, conexiones— antes que lo que se toca, y el reloj el
            // último, pegado a la esquina.
            derecha: vec![
                // El primero de la fila: es el único que dice *dónde estás* y
                // no *qué tienes*, así que va aparte de los estados, pegado al
                // borde izquierdo del grupo.
                "escritorios".into(),
                "bateria".into(),
                "bluetooth".into(),
                "red".into(),
                "volumen".into(),
                "brillo".into(),
                "notificaciones".into(),
                "control".into(),
                "reloj".into(),
            ],
            dock: [
                // El primero, como en el dock de macOS: es el cajón de todo lo
                // demás y conviene que esté donde siempre.
                (LAUNCHPAD, "Aplicaciones", "launchpad-cohete"),
                // El mismo por defecto que Meta+Return (`keybinds::TERMINAL`):
                // `bookos-shell` —lo que había aquí antes— es la biblioteca del
                // propio panel, no un programa que lanzar. No se lee
                // `BOOKOS_TERMINAL` aquí: el exec de un lanzador del dock pasa
                // por el validador de `ajustes` (`GetConfig`/`ApplyConfig`),
                // que rechaza `$`, `{` y `}` por ser metacaracteres de shell.
                ("konsole", "Terminal", "utilities-terminal"),
                ("bookos-explorer", "Archivos", "bookos-explorer"),
                ("firefox", "Navegador", "firefox"),
                // Los ajustes son los de BookOS, no los de Plasma: abrir el
                // panel de otro escritorio desde este es enseñar opciones que
                // no gobiernan lo que se está usando.
                ("bookos-settings", "Ajustes", "bookos-settings"),
                ("bookos-notepad", "Notas", "bookos-notepad"),
            ]
            .into_iter()
            .map(|(e, l, i)| Lanzador {
                exec: e.to_string(),
                etiqueta: l.to_string(),
                icono: i.to_string(),
                app_id: e.to_string(),
            })
            .collect(),
            escala: None,
            // 24 lógicos: el de KDE y el de GNOME, y el que asumen los temas
            // al elegir imagen. Pedir 24 a escala 1,75 da los 42 px que el
            // tema tiene dibujados de verdad, sin inventar píxeles.
            cursor: 24,
            dock_tamano: 50,
            bloqueo_huella: false,
            launchpad_dock: true,
            entrada: Entrada::default(),
            fondo: None,
            fondo_claro: None,
            fondo_oscuro: None,
            teclado: None,
            // Los valores de KDE, que eran los fijos antes de poder elegirlos.
            teclado_retardo: 600,
            teclado_velocidad: 25,
            pasos: Pasos::default(),
            escritorios: 2,
            nombres_escritorios: vec!["Escritorio 1".into(), "Escritorio 2".into()],
            tema: crate::tema::Tema::Oscuro,
            modo_tema: crate::tema::ModoTema::Oscuro,
            // Las siete y las ocho: ni el amanecer ni el anochecer de ningún
            // sitio en concreto, pero sí el tramo en el que la mayoría tiene
            // luz en la habitación.
            tema_claro_desde: (7, 0),
            tema_oscuro_desde: (20, 0),
            efectos: Efectos::default(),
            alto_contraste: false,
            acento: crate::tema::Acento::Azul,
            avatar: None,
            bloqueo: Bloqueo::default(),
            bloqueo_inactividad: 900,
            atenuacion_inactividad: 300,
            salvapantallas_inactividad: 1200,
            salvapantallas_estilo: EstiloSalvapantallas::Estela,
            apagado_inactividad: 1800,
            suspension_inactividad: 0,
            actividades: Actividades::default(),
            brillo_automatico: BrilloAutomatico::default(),
            luz_nocturna: LuzNocturna::default(),
            sonido_volumen: true,
            sonidos_interfaz: true,
            genio: true,
            panel_esquiva: false,
            dock_esquiva: false,
            iconos_escritorio: true,
        }
    }
}

impl Config {
    pub fn cargar() -> Self {
        let Some(ruta) = ruta() else {
            return Self::default();
        };
        let Ok(texto) = std::fs::read_to_string(&ruta) else {
            return Self::default();
        };
        Self::desde_texto(&texto, &ruta)
    }

    /// El análisis, separado de la lectura para poder probarlo sin tocar el
    /// disco: es donde están todas las decisiones que pueden salir mal.
    pub fn desde_texto(texto: &str, ruta: &std::path::Path) -> Self {
        let mut config = Self::default();
        for (n, linea) in texto.lines().enumerate() {
            let linea = linea.trim();
            if linea.is_empty() || linea.starts_with('#') {
                continue;
            }
            let Some((clave, valor)) = linea.split_once('=') else {
                tracing::warn!(?ruta, linea = n + 1, "línea sin '=', se ignora");
                continue;
            };
            let items = || {
                valor
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            };
            match clave.trim() {
                // "centro = " a secas es la forma de quitar el reloj.
                "centro" => config.centro = items().into_iter().next(),
                "derecha" => config.derecha = items(),
                "dock" => config.dock = items().iter().filter_map(|s| lanzador(s)).collect(),
                "dock_tamano" => {
                    if let Ok(n) = valor.trim().parse::<u32>() {
                        config.dock_tamano = n.clamp(32, 80);
                    }
                }
                "bloqueo_huella" => {
                    config.bloqueo_huella = matches!(valor.trim(), "si" | "sí" | "true" | "1");
                }
                // Un valor absurdo se descarta en vez de aplicarse: una escala
                // de 0 deja la pantalla en 0x0 píxeles lógicos y el escritorio
                // no vuelve a arrancar hasta editar el fichero a ciegas.
                "escritorios" => match valor.trim().parse::<usize>() {
                    Ok(v) => config.escritorios = v.clamp(1, MAXIMO_ESCRITORIOS),
                    Err(_) => tracing::warn!(valor, "«escritorios» no es un número"),
                },
                "nombres_escritorios" => config.nombres_escritorios = items(),
                // Sin comprobar que exista: el fichero puede llegar después que
                // la configuración —un montaje de red, por ejemplo— y descartar
                // la ruta aquí obligaría a editar el fichero otra vez.
                "avatar" => config.avatar = Some(valor.trim().to_string()),
                "launchpad_dock" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.launchpad_dock = v;
                    }
                }
                "bloqueo_animaciones" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.animaciones = v;
                    }
                }
                "actividades" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.actividades.habilitadas = v;
                    }
                }
                "actividades_animaciones" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.actividades.animaciones = v;
                    }
                }
                "genio" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.genio = v;
                    }
                }
                "sonido_volumen" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.sonido_volumen = v;
                    }
                }
                "sonidos_interfaz" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.sonidos_interfaz = v;
                    }
                }
                "panel_esquiva" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.panel_esquiva = v;
                    }
                }
                "dock_esquiva" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.dock_esquiva = v;
                    }
                }
                "iconos_escritorio" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.iconos_escritorio = v;
                    }
                }
                "temporizador_siempre_visible" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.actividades.temporizador_siempre = v;
                    }
                }
                "bloqueo_nombre" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.nombre = v;
                    }
                }
                "bloqueo_energia" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.energia = v;
                    }
                }
                "bloqueo_fecha" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.fecha = v;
                    }
                }
                "bloqueo_medios" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.medios = v;
                    }
                }
                "bloqueo_reloj_y" => {
                    if let Some(v) = decimal(valor, 0.02..=0.40, ruta, n + 1) {
                        config.bloqueo.reloj_y = v as f32;
                    }
                }
                "bloqueo_acceso_y" => {
                    if let Some(v) = decimal(valor, 0.18..=0.72, ruta, n + 1) {
                        config.bloqueo.acceso_y = v as f32;
                    }
                }
                "bloqueo_medios_y" => {
                    if let Some(v) = decimal(valor, 0.42..=0.88, ruta, n + 1) {
                        config.bloqueo.medios_y = v as f32;
                    }
                }
                "bloqueo_reloj_tamano" => {
                    if let Some(v) = decimal(valor, 72.0..=220.0, ruta, n + 1) {
                        config.bloqueo.reloj_tamano = v as f32;
                    }
                }
                "bloqueo_avatar_tamano" => {
                    if let Some(v) = decimal(valor, 64.0..=220.0, ruta, n + 1) {
                        config.bloqueo.avatar_tamano = v as f32;
                    }
                }
                "salvapantallas_estilo" => match EstiloSalvapantallas::desde_nombre(valor.trim()) {
                    Some(e) => config.salvapantallas_estilo = e,
                    None => tracing::warn!(
                        valor = valor.trim(),
                        "«salvapantallas_estilo» debe ser estela o rebote"
                    ),
                },
                "bloqueo_reloj_fuente" => match FuenteReloj::desde_nombre(valor.trim()) {
                    Some(f) => config.bloqueo.reloj_fuente = f,
                    None => tracing::warn!(
                        valor = valor.trim(),
                        "«bloqueo_reloj_fuente» no es un estilo de reloj conocido"
                    ),
                },
                "bloqueo_reloj_grosor" => match valor.trim().parse::<u16>() {
                    Ok(v) if (100..=900).contains(&v) && v % 100 == 0 => {
                        config.bloqueo.reloj_grosor = v
                    }
                    _ => tracing::warn!(
                        valor = valor.trim(),
                        "«bloqueo_reloj_grosor» debe ser 100, 200… hasta 900"
                    ),
                },
                "bloqueo_inactividad" => match valor.trim().parse::<u64>() {
                    Ok(v) if v <= 86_400 => config.bloqueo_inactividad = v,
                    _ => tracing::warn!(
                        valor = valor.trim(),
                        "«bloqueo_inactividad» debe estar entre 0 y 86400 segundos"
                    ),
                },
                "salvapantallas_inactividad" => match valor.trim().parse::<u64>() {
                    Ok(v) if v <= 86_400 => config.salvapantallas_inactividad = v,
                    _ => tracing::warn!(
                        valor = valor.trim(),
                        "«salvapantallas_inactividad» debe estar entre 0 y 86400 segundos"
                    ),
                },
                "atenuacion_inactividad" => match valor.trim().parse::<u64>() {
                    Ok(v) if v <= 86_400 => config.atenuacion_inactividad = v,
                    _ => tracing::warn!(
                        valor = valor.trim(),
                        "«atenuacion_inactividad» debe estar entre 0 y 86400 segundos"
                    ),
                },
                "apagado_inactividad" => match valor.trim().parse::<u64>() {
                    Ok(v) if v <= 86_400 => config.apagado_inactividad = v,
                    _ => tracing::warn!(
                        valor = valor.trim(),
                        "«apagado_inactividad» debe estar entre 0 y 86400 segundos"
                    ),
                },
                "suspension_inactividad" => match valor.trim().parse::<u64>() {
                    Ok(v) if v <= 86_400 => config.suspension_inactividad = v,
                    _ => tracing::warn!(
                        valor = valor.trim(),
                        "«suspension_inactividad» debe estar entre 0 y 86400 segundos"
                    ),
                },
                // Cualquier otra cosa se queda en oscuro y se avisa: un tema
                // mal escrito no puede dejar el escritorio a medio pintar.
                "tema" => match valor.trim() {
                    "claro" => config.modo_tema = crate::tema::ModoTema::Claro,
                    "oscuro" => config.modo_tema = crate::tema::ModoTema::Oscuro,
                    "auto" | "automatico" | "automático" => {
                        config.modo_tema = crate::tema::ModoTema::Automatico
                    }
                    otro => tracing::warn!(otro, "«tema» solo entiende claro, oscuro o automatico"),
                },
                // Un nombre que no está en la tabla se ignora en vez de
                // dejar el escritorio con un acento a medias.
                "efectos" => match valor.trim() {
                    "completos" => config.efectos = Efectos::Completos,
                    "reducidos" => config.efectos = Efectos::Reducidos,
                    otro => tracing::warn!(otro, "«efectos» solo entiende completos o reducidos"),
                },
                "alto_contraste" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.alto_contraste = v;
                    }
                }
                "brillo_automatico_pantalla" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.brillo_automatico.pantalla = v;
                    }
                }
                "brillo_automatico_teclado" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.brillo_automatico.teclado = v;
                    }
                }
                "luz_nocturna" => match valor.trim() {
                    "no" => config.luz_nocturna.modo = ModoLuzNocturna::Apagada,
                    "si" | "sí" => config.luz_nocturna.modo = ModoLuzNocturna::Encendida,
                    "programada" => config.luz_nocturna.modo = ModoLuzNocturna::Programada,
                    otro => {
                        tracing::warn!(otro, "«luz_nocturna» solo entiende no, si o programada")
                    }
                },
                "luz_nocturna_temperatura" => match valor.trim().parse::<u32>() {
                    Ok(v) if (LuzNocturna::MINIMA..=LuzNocturna::MAXIMA).contains(&v) => {
                        config.luz_nocturna.temperatura = v
                    }
                    _ => tracing::warn!(
                        ?ruta,
                        linea = n + 1,
                        valor = valor.trim(),
                        "temperatura fuera de [2500 - 8000] K; se ignora"
                    ),
                },
                "luz_nocturna_desde" => {
                    if let Some(h) = hora_del_dia(valor, ruta, n + 1) {
                        config.luz_nocturna.desde = h;
                    }
                }
                "luz_nocturna_hasta" => {
                    if let Some(h) = hora_del_dia(valor, ruta, n + 1) {
                        config.luz_nocturna.hasta = h;
                    }
                }
                "acento" => match crate::tema::Acento::desde_nombre(valor) {
                    Some(a) => config.acento = a,
                    None => tracing::warn!(valor = valor.trim(), "acento desconocido"),
                },
                "escala" => match valor.trim().replace(',', ".").parse::<f64>() {
                    Ok(v) if (0.5..=4.0).contains(&v) => config.escala = Some(v),
                    _ => tracing::warn!(
                        ?ruta,
                        linea = n + 1,
                        valor = valor.trim(),
                        "escala fuera de [0,5 - 4]; se ignora"
                    ),
                },
                // El límite de arriba no es un capricho: un cursor enorme deja
                // de caber en el plano de hardware del cursor y pasa a
                // componerse en la GPU, con lo que mover el ratón vuelve a
                // costar un frame entero.
                "cursor" => match valor.trim().parse::<u32>() {
                    Ok(v) if (8..=128).contains(&v) => config.cursor = v,
                    _ => tracing::warn!(
                        ?ruta,
                        linea = n + 1,
                        valor = valor.trim(),
                        "tamaño de cursor fuera de [8 - 128]; se ignora"
                    ),
                },
                "tema_claro_desde" => {
                    if let Some(h) = hora_del_dia(valor, ruta, n + 1) {
                        config.tema_claro_desde = h;
                    }
                }
                "tema_oscuro_desde" => {
                    if let Some(h) = hora_del_dia(valor, ruta, n + 1) {
                        config.tema_oscuro_desde = h;
                    }
                }
                // Vacío es «no hay», no «la ruta vacía»: un fichero editado a
                // mano con `fondo =` suelto no puede dejar el escritorio
                // buscando una imagen sin nombre.
                "fondo" => config.fondo = no_vacio(valor),
                "fondo_claro" => config.fondo_claro = no_vacio(valor),
                "fondo_oscuro" => config.fondo_oscuro = no_vacio(valor),
                "teclado" => config.teclado = Some(valor.trim().to_string()),
                "teclado_retardo" => {
                    if let Ok(n) = valor.trim().parse::<u32>() {
                        config.teclado_retardo = n.clamp(150, 1500);
                    }
                }
                "teclado_velocidad" => {
                    if let Ok(n) = valor.trim().parse::<u32>() {
                        config.teclado_velocidad = n.min(60);
                    }
                }
                "volumen_paso"
                | "volumen_paso_mantener"
                | "brillo_paso"
                | "brillo_paso_mantener" => {
                    if let Ok(n) = valor.trim().parse::<u8>() {
                        let n = n.clamp(
                            Pasos::MINIMO,
                            Pasos::maximo_de(clave.trim()).unwrap_or(Pasos::MAXIMO_PULSAR),
                        );
                        match clave.trim() {
                            "volumen_paso" => config.pasos.volumen = n,
                            "volumen_paso_mantener" => config.pasos.volumen_mantener = n,
                            "brillo_paso" => config.pasos.brillo = n,
                            _ => config.pasos.brillo_mantener = n,
                        }
                    }
                }
                "velocidad_touchpad" => {
                    if let Some(v) = velocidad(valor, ruta, n + 1) {
                        config.entrada.velocidad_touchpad = v;
                    }
                }
                "velocidad_raton" => {
                    if let Some(v) = velocidad(valor, ruta, n + 1) {
                        config.entrada.velocidad_raton = v;
                    }
                }
                "toque_para_clic" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.entrada.toque_para_clic = v;
                    }
                }
                "scroll_natural" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.entrada.scroll_natural = v;
                    }
                }
                "pantalla_tactil" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.entrada.pantalla_tactil = v;
                    }
                }
                otro => tracing::warn!(?ruta, linea = n + 1, clave = otro, "clave desconocida"),
            }
        }
        config.nombres_escritorios.truncate(config.escritorios);
        while config.nombres_escritorios.len() < config.escritorios {
            config.nombres_escritorios.push(format!(
                "Escritorio {}",
                config.nombres_escritorios.len() + 1
            ));
        }
        // El tema efectivo se deriva aquí y en ningún otro sitio: con el modo
        // automático depende de la hora, y tener dos verdades —lo elegido y lo
        // pintado— solo funciona si una sale de la otra en un único punto.
        config.tema = config.tema_ahora();
        config
    }

    /// El tema que toca en este instante, según el modo y el reloj.
    ///
    /// Se vuelve a llamar cuando el temporizador del compositor despierta en la
    /// hora del cambio, que es lo que hace que el escritorio se ponga oscuro
    /// solo al anochecer.
    pub fn tema_ahora(&self) -> crate::tema::Tema {
        self.modo_tema.resolver(
            self.tema_claro_desde,
            self.tema_oscuro_desde,
            crate::state::hora_local_ahora(),
        )
    }

    /// Cuánto falta para el próximo cambio automático de tema, si va a haberlo.
    pub fn hasta_el_cambio_de_tema(&self) -> Option<std::time::Duration> {
        self.modo_tema
            .minutos_al_cambio(
                self.tema_claro_desde,
                self.tema_oscuro_desde,
                crate::state::hora_local_ahora(),
            )
            .map(|min| std::time::Duration::from_secs(min as u64 * 60))
    }
}

/// Una velocidad de puntero en la escala de libinput. Fuera de [-1, 1]
/// libinput rechaza el valor y el dispositivo se queda como estaba, así que
/// más vale avisar aquí que dejar que falle en silencio al aplicarlo.
/// Una hora del reloj en `HH:MM`. También vale `7` a secas, que es `07:00`.
fn hora_del_dia(
    valor: &str,
    ruta: &std::path::Path,
    linea: usize,
) -> Option<crate::tema::HoraDelDia> {
    let texto = valor.trim();
    let partes = match texto.split_once(':') {
        Some((h, m)) => h.trim().parse::<u8>().ok().zip(m.trim().parse::<u8>().ok()),
        None => texto.parse::<u8>().ok().map(|h| (h, 0)),
    };
    // 24:00 no existe: el día acaba en 23:59 y quien quiera «medianoche» pone
    // 0:00. Admitirlo daría un tramo que empieza después de acabarse.
    match partes {
        Some((h, m)) if h < 24 && m < 60 => Some((h, m)),
        _ => {
            tracing::warn!(?ruta, linea, valor = texto, "hora no válida; se ignora");
            None
        }
    }
}

fn velocidad(valor: &str, ruta: &std::path::Path, linea: usize) -> Option<f64> {
    match valor.trim().replace(',', ".").parse::<f64>() {
        Ok(v) if (-1.0..=1.0).contains(&v) => Some(v),
        _ => {
            tracing::warn!(
                ?ruta,
                linea,
                valor = valor.trim(),
                "velocidad fuera de [-1 - 1]; se ignora"
            );
            None
        }
    }
}

fn decimal(
    valor: &str,
    rango: std::ops::RangeInclusive<f64>,
    ruta: &std::path::Path,
    linea: usize,
) -> Option<f64> {
    match valor.trim().replace(',', ".").parse::<f64>() {
        Ok(v) if rango.contains(&v) => Some(v),
        _ => {
            tracing::warn!(
                ?ruta,
                linea,
                valor = valor.trim(),
                minimo = *rango.start(),
                maximo = *rango.end(),
                "valor fuera de rango; se ignora"
            );
            None
        }
    }
}

/// El valor, o `None` si está en blanco.
fn no_vacio(valor: &str) -> Option<String> {
    let v = valor.trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn booleano(valor: &str, ruta: &std::path::Path, linea: usize) -> Option<bool> {
    match valor.trim() {
        "si" | "sí" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        otro => {
            tracing::warn!(?ruta, linea, valor = otro, "se esperaba sí/no; se ignora");
            None
        }
    }
}

/// `exec:etiqueta:icono:app_id`, y todo menos `exec` se puede omitir.
///
/// Lo que falta se deduce del ejecutable, que es lo que acierta la mayoría de
/// las veces: `firefox` a secas ya da el icono y el `app_id` correctos.
fn lanzador(spec: &str) -> Option<Lanzador> {
    let mut campos = spec.split(':').map(str::trim);
    let exec = campos.next().filter(|s| !s.is_empty())?;
    let o_exec = |c: Option<&str>| c.filter(|s| !s.is_empty()).unwrap_or(exec).to_string();
    Some(Lanzador {
        etiqueta: o_exec(campos.next()),
        icono: o_exec(campos.next()),
        app_id: o_exec(campos.next()),
        exec: exec.to_string(),
    })
}

/// Reescribe la clave `dock` del fichero, dejando lo demás como está.
pub fn guardar_dock(anclados: &[String]) -> std::io::Result<()> {
    escribir_claves(&[("dock", anclados.join(", "))])
}

/// Persiste la cantidad y los nombres sin reescribir el resto del fichero.
pub fn guardar_escritorios(nombres: &[String]) -> std::io::Result<()> {
    // La coma separa los nombres en el fichero, así que un nombre con coma
    // partiría la lista en dos escritorios al releerla.
    let nombres: Vec<_> = nombres
        .iter()
        .map(|n| n.replace([',', '\n', '\r'], " "))
        .collect();
    escribir_claves(&[
        ("escritorios", nombres.len().to_string()),
        ("nombres_escritorios", nombres.join(", ")),
    ])
}

/// Persiste el tema y el acento que se acaban de elegir en Apariencia.
/// Guarda lo que el usuario **eligió**, no el tema que se está pintando.
///
/// Con el modo automático son cosas distintas: a las once de la noche lo
/// pintado es oscuro y lo elegido es «automático», y escribir `oscuro` en el
/// fichero convertiría el automático en fijo al reiniciar la sesión.
pub fn guardar_apariencia(
    modo: crate::tema::ModoTema,
    acento: crate::tema::Acento,
) -> std::io::Result<()> {
    let tema = match modo {
        crate::tema::ModoTema::Claro => "claro",
        crate::tema::ModoTema::Oscuro => "oscuro",
        crate::tema::ModoTema::Automatico => "automatico",
    };
    escribir_claves(&[
        ("tema", tema.to_string()),
        ("acento", acento.nombre().to_string()),
    ])
}

/// Persiste la pareja de fondos elegida.
///
/// Escribe `fondo_claro` y `fondo_oscuro` y **vacía** `fondo`: si se quedara, el
/// común mandaría sobre la pareja al volver a cargar y el cambio de tema dejaría
/// de llevarse la imagen con él, que es justo lo que se acaba de elegir.
pub fn guardar_fondo(claro: &std::path::Path, oscuro: &std::path::Path) -> std::io::Result<()> {
    escribir_claves(&[
        ("fondo", String::new()),
        ("fondo_claro", claro.to_string_lossy().into_owned()),
        ("fondo_oscuro", oscuro.to_string_lossy().into_owned()),
    ])
}

/// Quita las tres claves del fondo, dejando el fichero como si nunca se hubiera
/// elegido ninguno. La usa el autotest para devolver lo que encontró.
pub fn olvidar_fondo() -> std::io::Result<()> {
    escribir_claves(&[
        ("fondo", String::new()),
        ("fondo_claro", String::new()),
        ("fondo_oscuro", String::new()),
    ])
}

/// Persiste el modo de efectos elegido.
pub fn guardar_efectos(efectos: Efectos) -> std::io::Result<()> {
    let valor = match efectos {
        Efectos::Completos => "completos",
        Efectos::Reducidos => "reducidos",
    };
    escribir_claves(&[("efectos", valor.to_string())])
}

/// Persiste qué luces siguen al sensor.
pub fn guardar_brillo_automatico(elegido: BrilloAutomatico) -> std::io::Result<()> {
    let si_no = |v: bool| if v { "si" } else { "no" }.to_string();
    escribir_claves(&[
        ("brillo_automatico_pantalla", si_no(elegido.pantalla)),
        ("brillo_automatico_teclado", si_no(elegido.teclado)),
    ])
}

/// Persiste el modo de la luz nocturna, que es lo único que cambia desde la
/// baldosa y la tarjeta de brillo: la temperatura y el horario son de Ajustes.
pub fn guardar_luz_nocturna(modo: ModoLuzNocturna) -> std::io::Result<()> {
    let valor = match modo {
        ModoLuzNocturna::Apagada => "no",
        ModoLuzNocturna::Encendida => "si",
        ModoLuzNocturna::Programada => "programada",
    };
    escribir_claves(&[("luz_nocturna", valor.to_string())])
}

/// Persiste si una barra se aparta de las ventanas. `clave` es
/// `panel_esquiva` o `dock_esquiva`.
pub fn guardar_esquiva(clave: &str, esquiva: bool) -> std::io::Result<()> {
    escribir_claves(&[(clave, if esquiva { "si" } else { "no" }.to_string())])
}

/// Persiste si se ven los iconos del escritorio.
pub fn guardar_iconos_escritorio(visibles: bool) -> std::io::Result<()> {
    escribir_claves(&[(
        "iconos_escritorio",
        if visibles { "si" } else { "no" }.to_string(),
    )])
}

/// Punto único de persistencia para la API estructurada del compositor.
/// Las claves ya llegan validadas por `org.bookos.Desktop`; esta función
/// conserva comentarios, orden y claves que una versión nueva aún no conozca.
pub fn guardar_configuracion(valores: &[(String, String)]) -> std::io::Result<()> {
    let prestados: Vec<_> = valores
        .iter()
        .map(|(clave, valor)| (clave.as_str(), valor.clone()))
        .collect();
    escribir_claves(&prestados)
}

/// Reescribe esas claves del fichero y deja lo demás como está.
///
/// A mano y no serializando la `Config` entera porque el fichero es del
/// usuario: lleva sus comentarios y su orden, y volcarlo desde el código los
/// borraría. La clave que ya estaba se sustituye en su sitio; la que no,
/// se añade al final.
///
/// **Un valor vacío borra la clave** en vez de escribir `clave = `. Es lo que
/// hace falta para desdecirse: al elegir una pareja de fondos hay que quitar el
/// `fondo` común, y dejarlo con el valor en blanco no lo quita —se relee como
/// una ruta vacía y manda igual sobre la pareja.
fn escribir_claves(valores: &[(&str, String)]) -> std::io::Result<()> {
    let Some(ruta) = ruta() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let anterior = std::fs::read_to_string(&ruta).unwrap_or_default();
    let mut vistos = vec![false; valores.len()];
    let mut salida = String::new();
    for linea in anterior.lines() {
        let clave = linea.split_once('=').map(|(k, _)| k.trim());
        if let Some(i) = valores.iter().position(|(k, _)| Some(*k) == clave) {
            // Repetida en el fichero: la primera se sustituye y las demás se
            // caen, que es lo que hace la lectura —se queda con la última— al
            // revés, pero deja el fichero sin claves duplicadas.
            if !vistos[i] && !valores[i].1.is_empty() {
                salida.push_str(&format!("{} = {}", valores[i].0, valores[i].1));
                vistos[i] = true;
            } else {
                // Vacía: la línea se cae y la clave desaparece del fichero.
                vistos[i] = true;
                continue;
            }
        } else {
            salida.push_str(linea);
        }
        salida.push('\n');
    }
    for (i, (clave, valor)) in valores.iter().enumerate() {
        // La que no estaba se añade al final; la vacía no se añade: borrar una
        // clave que no existía es no hacer nada.
        if !vistos[i] && !valor.is_empty() {
            salida.push_str(&format!("{clave} = {valor}\n"));
        }
    }
    std::fs::write(ruta, salida)
}

/// El launchpad guardado, en `~/.config/bookos/launchpad.conf`.
///
/// Un fichero aparte y no una clave de `panel.conf` porque esto **lo escribe el
/// escritorio**, no el usuario: cada vez que se arrastra un icono se reescribe
/// entero, y mezclarlo con la configuración escrita a mano acabaría pisando los
/// comentarios de alguien.
///
/// Una carpeta por línea, `nombre[:color] = exec1, exec2, …`, más la lista de
/// las aplicaciones que se han quitado de la rejilla:
///
/// ```text
/// Utilidades:verde = konsole, kate, kcalc
/// Internet = firefox, thunderbird
/// ocultas = xterm, gnome-tetravex
/// orden = kate, firefox, konsole
/// ```
///
/// El color es uno de la tabla de [`crate::tema::Acento`]; sin él, la carpeta
/// va en gris. [`CLAVE_OCULTAS`] es una clave reservada: una carpeta no puede
/// llamarse así.
///
/// Las aplicaciones se identifican por su `exec` porque es lo que ya usa el
/// dock en `panel.conf`: dos formas de nombrar la misma aplicación en el mismo
/// escritorio se separan a la primera.
pub const CLAVE_OCULTAS: &str = "ocultas";

/// El orden manual de las aplicaciones sueltas, para moverlas de página. Es
/// clave reservada por lo mismo que [`CLAVE_OCULTAS`].
pub const CLAVE_ORDEN: &str = "orden";

/// Lo que hay en `launchpad.conf`.
#[derive(Default)]
pub struct Launchpad {
    /// Nombre, color —si lo tiene— y aplicaciones de cada carpeta.
    pub carpetas: Vec<(String, Option<String>, Vec<String>)>,
    /// Las que el usuario ha quitado de la rejilla.
    pub ocultas: Vec<String>,
    /// Las sueltas que el usuario ha colocado a mano, en su orden.
    pub orden: Vec<String>,
}

pub fn cargar_launchpad() -> Launchpad {
    let Some(ruta) = ruta_launchpad() else {
        return Launchpad::default();
    };
    let Ok(texto) = std::fs::read_to_string(&ruta) else {
        return Launchpad::default();
    };
    interpretar_launchpad(&texto)
}

fn interpretar_launchpad(texto: &str) -> Launchpad {
    let mut salida = Launchpad::default();
    for linea in texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Some((clave, valores)) = linea.split_once('=') else {
            continue;
        };
        let valores: Vec<String> = valores
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        // Una carpeta vacía no se enseña: sería un icono que al abrirse no
        // tiene nada dentro.
        if valores.is_empty() {
            continue;
        }
        let clave = clave.trim();
        if clave == CLAVE_OCULTAS {
            salida.ocultas = valores;
            continue;
        }
        if clave == CLAVE_ORDEN {
            salida.orden = valores;
            continue;
        }
        let (nombre, color) = match clave.split_once(':') {
            Some((n, c)) => (n.trim(), Some(c.trim().to_string())),
            None => (clave, None),
        };
        salida.carpetas.push((nombre.to_string(), color, valores));
    }
    salida
}

/// Reescribe el fichero entero. Lo llama el launchpad al cambiar algo.
pub fn guardar_launchpad(datos: &Launchpad) -> std::io::Result<()> {
    let Some(ruta) = ruta_launchpad() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut salida = String::from(
        "# El launchpad: carpetas y aplicaciones quitadas de la rejilla.\n\
         # Lo escribe el escritorio; se puede editar a mano.\n\
         # Una carpeta por línea: nombre[:color] = exec1, exec2, …\n",
    );
    for (nombre, color, apps) in &datos.carpetas {
        // Ni `=`, ni `,`, ni `:` en el nombre: los tres partirían la línea al
        // volver a leerla. Se sustituyen en vez de perder la carpeta entera.
        let nombre = nombre.replace(['=', ',', ':', '\n', '\r'], " ");
        let nombre = nombre.trim();
        match color {
            Some(color) => salida.push_str(&format!("{nombre}:{color} = {}\n", apps.join(", "))),
            None => salida.push_str(&format!("{nombre} = {}\n", apps.join(", "))),
        }
    }
    if !datos.ocultas.is_empty() {
        salida.push_str(&format!("{CLAVE_OCULTAS} = {}\n", datos.ocultas.join(", ")));
    }
    if !datos.orden.is_empty() {
        salida.push_str(&format!("{CLAVE_ORDEN} = {}\n", datos.orden.join(", ")));
    }
    std::fs::write(ruta, salida)
}

/// Cuántas veces se ha lanzado cada `exec`, en `~/.config/bookos/usos.conf`.
/// Aparte de `launchpad.conf` porque lo escribe el compositor en cada
/// lanzamiento, venga del dock, del buscador o del launchpad, y aquel lo
/// reescribe el launchpad entero con lo que tenía en memoria.
///
/// Una línea por aplicación, `n = exec`: el número a la izquierda porque un
/// `exec` puede llevar un igual dentro.
pub fn cargar_usos() -> HashMap<String, u32> {
    ruta_usos()
        .and_then(|ruta| std::fs::read_to_string(ruta).ok())
        .map(|texto| interpretar_usos(&texto))
        .unwrap_or_default()
}

fn interpretar_usos(texto: &str) -> HashMap<String, u32> {
    texto
        .lines()
        .filter_map(|l| l.split_once('='))
        .filter_map(|(n, exec)| Some((exec.trim().to_string(), n.trim().parse().ok()?)))
        .filter(|(exec, _)| !exec.is_empty())
        .collect()
}

pub fn sumar_uso(exec: &str) -> std::io::Result<()> {
    let Some(ruta) = ruta_usos() else {
        return Ok(());
    };
    let mut usos = cargar_usos();
    *usos.entry(exec.trim().to_string()).or_default() += 1;
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let salida: String = usos
        .iter()
        .map(|(exec, n)| format!("{n} = {exec}\n"))
        .collect();
    std::fs::write(ruta, salida)
}

/// Las aplicaciones que el usuario quiere siempre encima, en
/// `~/.config/bookos/encima.conf`, un `app_id` por línea. Se lee al abrir cada
/// ventana —pasa pocas veces y así no hay copia en memoria que desincronizar—.
pub fn apps_encima() -> Vec<String> {
    ruta_encima()
        .and_then(|ruta| std::fs::read_to_string(ruta).ok())
        .map(|texto| {
            texto
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Añade o quita `app_id` de [`apps_encima`].
pub fn marcar_app_encima(app_id: &str, encima: bool) -> std::io::Result<()> {
    let Some(ruta) = ruta_encima() else {
        return Ok(());
    };
    let mut apps = apps_encima();
    apps.retain(|a| a != app_id);
    if encima {
        apps.push(app_id.to_string());
    }
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        ruta,
        apps.iter().map(|a| format!("{a}\n")).collect::<String>(),
    )
}

fn ruta_encima() -> Option<PathBuf> {
    Some(ruta()?.with_file_name("encima.conf"))
}

fn ruta_usos() -> Option<PathBuf> {
    Some(ruta()?.with_file_name("usos.conf"))
}

fn ruta_launchpad() -> Option<PathBuf> {
    Some(ruta()?.with_file_name("launchpad.conf"))
}

/// Dónde ha dejado el usuario cada icono del escritorio.
///
/// El fichero es `~/.config/bookos/escritorio.conf` y cada línea es
/// `columna,fila = nombre`. Las coordenadas van a la **izquierda** del `=` a
/// propósito: un nombre de fichero puede llevar un igual dentro, y al revés la
/// línea se partiría por donde no toca.
pub fn cargar_escritorio() -> Vec<(String, (i32, i32))> {
    let Some(ruta) = ruta_escritorio() else {
        return Vec::new();
    };
    let Ok(texto) = std::fs::read_to_string(&ruta) else {
        return Vec::new();
    };
    texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|linea| {
            let (celda, nombre) = linea.split_once('=')?;
            let (col, fila) = celda.trim().split_once(',')?;
            Some((
                nombre.trim().to_string(),
                (col.trim().parse().ok()?, fila.trim().parse().ok()?),
            ))
        })
        .collect()
}

/// Reescribe las posiciones enteras. Lo llama el escritorio al soltar un icono.
pub fn guardar_escritorio(posiciones: &[(&str, (i32, i32))]) -> std::io::Result<()> {
    let Some(ruta) = ruta_escritorio() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut salida = String::from("# Dónde va cada icono del escritorio: columna,fila = nombre.\n");
    for (nombre, (col, fila)) in posiciones {
        // Un salto de línea en el nombre partiría el fichero en dos entradas.
        let nombre = nombre.replace(['\n', '\r'], " ");
        salida.push_str(&format!("{col},{fila} = {nombre}\n"));
    }
    std::fs::write(ruta, salida)
}

fn ruta_escritorio() -> Option<PathBuf> {
    Some(ruta()?.with_file_name("escritorio.conf"))
}

pub fn ruta() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("bookos/panel.conf"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/bookos/panel.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_usos_admiten_un_igual_en_el_exec() {
        let usos =
            interpretar_usos("3 = bookos-settings --page=fondos\nbasura\nx = kate\n7 = firefox\n");
        assert_eq!(usos.get("bookos-settings --page=fondos"), Some(&3));
        assert_eq!(usos.get("firefox"), Some(&7));
        assert_eq!(usos.len(), 2, "lo que no es un número se ignora");
    }

    fn parsear(texto: &str) -> Config {
        Config::desde_texto(texto, std::path::Path::new("prueba.conf"))
    }

    #[test]
    fn dock_y_huella_tienen_valores_seguros() {
        assert_eq!(parsear("").dock_tamano, 50);
        assert!(!parsear("").bloqueo_huella);
        assert_eq!(parsear("dock_tamano = 12").dock_tamano, 32);
        assert_eq!(parsear("dock_tamano = 120").dock_tamano, 80);
        assert_eq!(parsear("dock_tamano = absurdo").dock_tamano, 50);
        assert!(parsear("bloqueo_huella = si").bloqueo_huella);
        assert!(!parsear("bloqueo_huella = quizá").bloqueo_huella);
    }

    #[test]
    fn los_pasos_se_recortan_a_su_techo() {
        let c = parsear(
            "volumen_paso = 99\nbrillo_paso = 0\nvolumen_paso_mantener = 99\nbrillo_paso_mantener = 0",
        );
        assert_eq!(c.pasos.volumen, 10, "al pulsar, hasta 10");
        assert_eq!(c.pasos.brillo, 1);
        assert_eq!(c.pasos.volumen_mantener, 30, "al mantener, hasta 30");
        assert_eq!(c.pasos.brillo_mantener, 1);
        assert_eq!(parsear("volumen_paso = 7").pasos.volumen, 7);
        assert_eq!(
            parsear("volumen_paso = x").pasos.volumen,
            5,
            "ilegible: el de siempre"
        );
    }

    #[test]
    fn las_barras_recuerdan_si_esquivan() {
        let c = parsear("");
        assert!(
            !c.panel_esquiva && !c.dock_esquiva,
            "de serie, siempre a la vista"
        );
        let c = parsear("panel_esquiva = si\ndock_esquiva = no");
        assert!(c.panel_esquiva);
        assert!(!c.dock_esquiva);
    }

    #[test]
    fn sin_fichero_manda_lo_de_serie() {
        let c = parsear("");
        assert_eq!(c.entrada.velocidad_touchpad, 0.3);
        assert!(c.entrada.toque_para_clic);
        assert!(c.entrada.scroll_natural);
        assert!(c.entrada.pantalla_tactil);
        assert_eq!(c.escala, None);
        assert_eq!(c.escritorios, 2);
        assert_eq!(c.nombres_escritorios, ["Escritorio 1", "Escritorio 2"]);
    }

    #[test]
    fn escritorios_se_acotan_y_los_nombres_se_completan() {
        let c = parsear("escritorios = 9\nnombres_escritorios = Trabajo, Juegos\n");
        assert_eq!(c.escritorios, 5);
        assert_eq!(c.nombres_escritorios.len(), 5);
        assert_eq!(&c.nombres_escritorios[..2], ["Trabajo", "Juegos"]);
        assert_eq!(parsear("escritorios = 0").escritorios, 1);
    }

    #[test]
    fn la_entrada_se_lee_entera() {
        let c = parsear(
            "velocidad_touchpad = 0.8\n\
             velocidad_raton = -0.2\n\
             toque_para_clic = no\n\
             scroll_natural = no\n\
             pantalla_tactil = no\n",
        );
        assert_eq!(c.entrada.velocidad_touchpad, 0.8);
        assert_eq!(c.entrada.velocidad_raton, -0.2);
        assert!(!c.entrada.toque_para_clic);
        assert!(!c.entrada.scroll_natural);
        assert!(!c.entrada.pantalla_tactil);
    }

    /// Escribir la coma decimal es lo natural en castellano y no debería
    /// costar una velocidad ignorada; ya se aceptaba en `escala`.
    #[test]
    fn la_coma_decimal_vale() {
        let c = parsear("velocidad_touchpad = 0,5");
        assert_eq!(c.entrada.velocidad_touchpad, 0.5);
    }

    /// libinput rechaza lo que se salga de [-1, 1] y deja el dispositivo como
    /// estaba, así que una errata tiene que quedarse en el valor de serie y no
    /// en un silencio.
    #[test]
    fn una_velocidad_imposible_no_se_aplica() {
        let alta = parsear("velocidad_touchpad = 5");
        assert_eq!(alta.entrada.velocidad_touchpad, 0.3);
        let absurda = parsear("velocidad_raton = mucha");
        assert_eq!(absurda.entrada.velocidad_raton, 0.0);
    }

    #[test]
    fn el_tamano_del_cursor_se_lee_y_se_acota() {
        assert_eq!(parsear("cursor = 32").cursor, 32);
        // Un cursor de 4 px no se ve y uno de 500 no cabe en el plano de
        // hardware: en ambos casos vale más quedarse con el de serie.
        assert_eq!(parsear("cursor = 4").cursor, 24);
        assert_eq!(parsear("cursor = 500").cursor, 24);
    }

    #[test]
    fn la_composicion_del_bloqueo_es_configurable_y_segura() {
        let c = parsear(
            "bloqueo_animaciones = no\n\
             bloqueo_fecha = no\n\
             bloqueo_medios = sí\n\
             bloqueo_reloj_y = 0.12\n\
             bloqueo_acceso_y = 0.42\n\
             bloqueo_medios_y = 0.73\n\
             bloqueo_reloj_tamano = 168\n\
             bloqueo_avatar_tamano = 150\n",
        );
        assert!(!c.bloqueo.animaciones);
        assert!(!c.bloqueo.fecha);
        assert!(c.bloqueo.medios);
        assert_eq!(c.bloqueo.reloj_y, 0.12);
        assert_eq!(c.bloqueo.acceso_y, 0.42);
        assert_eq!(c.bloqueo.medios_y, 0.73);
        assert_eq!(c.bloqueo.reloj_tamano, 168.0);
        assert_eq!(c.bloqueo.avatar_tamano, 150.0);

        // Una posición fuera de pantalla no pisa el valor utilizable de serie.
        assert_eq!(parsear("bloqueo_acceso_y = 4").bloqueo.acceso_y, 0.36);
    }

    #[test]
    fn el_estilo_del_reloj_solo_acepta_la_lista_y_grosores_de_cien_en_cien() {
        let c = parsear("bloqueo_reloj_fuente = clasica\nbloqueo_reloj_grosor = 700\n");
        assert_eq!(c.bloqueo.reloj_fuente, FuenteReloj::Clasica);
        assert_eq!(c.bloqueo.reloj_grosor, 700);

        // Fuera de la lista o entre medias se queda lo de serie.
        let c = parsear("bloqueo_reloj_fuente = Comic Sans\nbloqueo_reloj_grosor = 450\n");
        assert_eq!(c.bloqueo.reloj_fuente, FuenteReloj::Moderna);
        assert_eq!(c.bloqueo.reloj_grosor, 400);
        assert_eq!(
            parsear("bloqueo_reloj_grosor = 1000").bloqueo.reloj_grosor,
            400
        );

        // El nombre del fichero y el del enum van y vuelven.
        for f in FuenteReloj::TODAS {
            assert_eq!(FuenteReloj::desde_nombre(f.nombre()), Some(f));
        }
    }

    #[test]
    fn la_inactividad_del_bloqueo_tiene_limites() {
        assert_eq!(
            parsear("bloqueo_inactividad = 600").bloqueo_inactividad,
            600
        );
        assert_eq!(parsear("bloqueo_inactividad = 0").bloqueo_inactividad, 0);
        assert_eq!(
            parsear("bloqueo_inactividad = 90001").bloqueo_inactividad,
            900
        );
        assert_eq!(parsear("bloqueo_inactividad = no").bloqueo_inactividad, 900);
        assert_eq!(
            parsear("suspension_inactividad = 1800").suspension_inactividad,
            1800
        );
        assert_eq!(
            parsear("suspension_inactividad = 90001").suspension_inactividad,
            0
        );
        assert_eq!(
            parsear("suspension_inactividad = no").suspension_inactividad,
            0
        );
    }

    #[test]
    fn las_actividades_y_su_movimiento_se_pueden_desactivar() {
        let c = parsear(
            "actividades = no\n\
             actividades_animaciones = no\n\
             temporizador_siempre_visible = sí\n",
        );
        assert!(!c.actividades.habilitadas);
        assert!(!c.actividades.animaciones);
        assert!(c.actividades.temporizador_siempre);
    }

    #[test]
    fn el_plazo_del_salvapantallas_admite_desactivarlo_y_rechaza_errores() {
        assert_eq!(parsear("").salvapantallas_inactividad, 1200);
        assert_eq!(parsear("").atenuacion_inactividad, 300);
        assert_eq!(
            parsear("").salvapantallas_estilo,
            EstiloSalvapantallas::Estela
        );
        assert_eq!(
            parsear("salvapantallas_estilo = rebote").salvapantallas_estilo,
            EstiloSalvapantallas::Rebote
        );
        assert_eq!(
            parsear("salvapantallas_estilo = burbujas").salvapantallas_estilo,
            EstiloSalvapantallas::Estela
        );
        assert_eq!(parsear("").apagado_inactividad, 1800);
        assert_eq!(
            parsear("apagado_inactividad = 600").apagado_inactividad,
            600
        );
        assert_eq!(
            parsear("salvapantallas_inactividad = 0").salvapantallas_inactividad,
            0
        );
        assert_eq!(
            parsear("salvapantallas_inactividad = 60").salvapantallas_inactividad,
            60
        );
        for valor in ["-1", "86401", "no"] {
            assert_eq!(
                parsear(&format!("salvapantallas_inactividad = {valor}"))
                    .salvapantallas_inactividad,
                1200
            );
        }
    }

    #[test]
    fn el_launchpad_va_y_vuelve() {
        let texto = "# comentario\n\
                     Utilidades:verde = konsole, kate , kcalc\n\
                     \n\
                     Vacia = \n\
                     Internet = firefox\n\
                     ocultas = xterm\n";
        let datos = interpretar_launchpad(texto);
        assert_eq!(datos.carpetas.len(), 2, "la vacía no cuenta");
        assert_eq!(datos.carpetas[0].0, "Utilidades");
        assert_eq!(datos.carpetas[0].1.as_deref(), Some("verde"), "el color");
        assert_eq!(datos.carpetas[0].2, ["konsole", "kate", "kcalc"]);
        assert_eq!(datos.carpetas[1].1, None, "sin color va en gris");
        assert_eq!(datos.ocultas, ["xterm"]);
    }

    #[test]
    fn el_acento_sale_de_la_tabla_y_lo_demas_se_ignora() {
        assert_eq!(
            parsear("acento = automatico").acento,
            crate::tema::Acento::Automatico
        );
        assert_eq!(
            parsear("acento = morado").acento,
            crate::tema::Acento::Morado
        );
        // Un acento inventado deja el de serie: el escritorio tiene que
        // arrancar con un color, no con ninguno.
        assert_eq!(parsear("acento = fucsia").acento, crate::tema::Acento::Azul);
    }

    #[test]
    fn el_booleano_entiende_si_y_no() {
        assert!(parsear("toque_para_clic = sí").entrada.toque_para_clic);
        assert!(!parsear("toque_para_clic = 0").entrada.toque_para_clic);
        // Lo que no entiende deja el valor de serie en vez de inventarse uno.
        assert!(parsear("toque_para_clic = quizá").entrada.toque_para_clic);
    }

    #[test]
    fn el_brillo_automatico_se_elige_por_luz() {
        assert_eq!(parsear("").brillo_automatico, BrilloAutomatico::default());
        let c = parsear("brillo_automatico_teclado = si");
        assert!(c.brillo_automatico.teclado);
        assert!(!c.brillo_automatico.pantalla);
    }

    /// Una línea mala no puede llevarse por delante a las demás: el escritorio
    /// tiene que arrancar igual.
    #[test]
    fn una_errata_no_tumba_el_resto() {
        let c = parsear("escala = 1.5\nesto no es nada\nvelocidad_touchpad = 0.4\n");
        assert_eq!(c.escala, Some(1.5));
        assert_eq!(c.entrada.velocidad_touchpad, 0.4);
    }
}
