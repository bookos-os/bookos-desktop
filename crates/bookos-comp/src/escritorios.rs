//! Escritorios virtuales.
//!
//! ## Por qué un solo `Space` y no uno por escritorio
//!
//! Lo evidente sería `Vec<Space<Window>>` y cambiar de índice. Pero el `Space`
//! se consulta desde 74 sitios repartidos por once ficheros —el render, el
//! hit-testing, el encaje, el foco, XWayland—, y todos dan por hecho que hay
//! **uno**. Cambiar eso es tocar el compositor entero para una feature que en
//! realidad solo necesita responder a una pregunta: qué ventanas se ven ahora.
//!
//! Aquí el `Space` sigue siendo uno y contiene **solo las ventanas del
//! escritorio activo**. Las de los demás se sacan con `unmap_elem` y se guardan
//! con su posición. Para el cliente no cambia nada: desmapear un elemento del
//! `Space` no le manda ningún evento, no le quita el buffer y no le cierra nada
//! —una ventana desmapeada simplemente no se dibuja ni se puede pulsar—. Y todo
//! el código que ya existía sigue viendo exactamente lo que hay en pantalla,
//! sin enterarse de que existen los escritorios.
//!
//! Una ventana solo está en un escritorio a la vez. La vista general puede
//! previsualizar también las apartadas porque conserva aquí tanto la `Window`
//! viva como su posición: no hace falta mapearlas en el `Space` para componerlas
//! escaladas.
//!
//! ## Por qué los escritorios son **por salida**
//!
//! Antes eran globales: `cambiar_a` se llevaba `space.elements()` entero, así
//! que cambiar de escritorio con el ratón en el portátil vaciaba también el
//! monitor externo. Y la animación usaba el ancho de la pantalla **bajo el
//! puntero**, de modo que con un monitor de 1280 al lado de uno de 1920 las
//! ventanas del grande se quedaban a media pantalla durante la transición.
//!
//! Ahora cada salida lleva su escritorio activo y su propia lista de guardadas,
//! como en macOS y en KWin: mover un escritorio en un monitor no toca el otro,
//! y cada uno desliza el ancho que le corresponde. Puede haber un cambio en
//! marcha en cada salida a la vez, y por eso `deslizando` es un mapa y no uno
//! solo.
//!
//! Lo que **no** es por salida: cuántos escritorios hay y cómo se llaman —eso
//! es un ajuste del usuario, no del monitor—. «Mostrar el escritorio» despeja
//! todas las pantallas a la vez porque es lo que pide el gesto, pero lo que
//! aparta se guarda en el escritorio de cada una: ver `Escritorios::escondidas`.
//!
//! Las salidas se identifican por su **nombre** (`Output::name`) y no por el
//! `Output` porque el mapa tiene que sobrevivir a un desenchufe y a que Smithay
//! recree el objeto: al volver a enchufar el mismo monitor, sus escritorios
//! siguen ahí.

use std::collections::HashMap;

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::BookosComp;

/// Cuántos escritorios hay como máximo.
///
/// El número de verdad sale de `panel.conf`; esto acota tanto la vista como los
/// atajos directos. Es el mismo tope que el del shell —hasta ahí llegan sus
/// iconos numerados— y sale de allí para que no puedan separarse.
pub const MAXIMO: usize = bookos_shell::MAXIMO_ESCRITORIOS;

/// Una ventana apartada, con dónde estaba.
///
/// La posición se guarda aquí porque `unmap_elem` la olvida: sin esto, volver a
/// un escritorio recolocaría todas sus ventanas en el origen.
pub type Apartada = (Window, Point<i32, Logical>);

/// Un cambio de escritorio en marcha.
///
/// Mientras dura, los **dos** juegos de ventanas están mapeados: las que se van
/// deslizándose hacia fuera y las que entran viniendo desde el otro lado. Es lo
/// que hace macOS, y no es un adorno: sin el deslizamiento, cambiar de
/// escritorio es un corte de plano y no queda ni rastro de hacia dónde te has
/// movido, que es justo lo que se necesita para no perderse entre cuatro.
#[derive(Debug)]
struct Deslizamiento {
    desde: std::time::Instant,
    /// Hacia dónde va la vista: `1` si vamos a un escritorio de la derecha.
    direccion: f64,
    /// Ancho de la pantalla en lógicos, que es lo que recorren las ventanas.
    ancho: f64,
    /// Las que se van, con la posición que tenían **antes** de empezar.
    salientes: Vec<Apartada>,
    /// Y las que entran, con la posición en la que tienen que acabar.
    entrantes: Vec<Apartada>,
}

/// Una ventana del despeje: ella, su sitio y el punto fuera de la pantalla.
type Viaje = (Window, Point<i32, Logical>, Point<i32, Logical>);

/// Las ventanas yéndose a los bordes al despejar, o volviendo de ellos.
#[derive(Debug)]
struct Despeje {
    desde: std::time::Instant,
    /// `true` al irse: al acabar se desmapean. Al volver ya acaban en su sitio.
    ida: bool,
    ventanas: Vec<Viaje>,
}

#[derive(Debug)]
pub struct Escritorios {
    /// El escritorio activo **de cada salida**, por nombre de salida. Una que
    /// no esté en el mapa está en el primero: así una pantalla recién
    /// enchufada no necesita que nadie la dé de alta.
    activo: HashMap<String, usize>,
    /// Cuántos hay, de `panel.conf`. Es el mismo número en todas las salidas.
    cuantos: usize,
    nombres: Vec<String>,
    /// Las ventanas guardadas: por salida, y dentro una lista por escritorio.
    /// La del escritorio activo de esa salida está siempre vacía —sus ventanas
    /// viven en el `Space`—.
    guardadas: HashMap<String, Vec<Vec<Apartada>>>,
    /// Las ventanas apartadas por «mostrar el escritorio», con la misma forma
    /// que `guardadas`: por salida y por escritorio.
    ///
    /// Antes era una sola lista global y cambiar de escritorio la devolvía
    /// entera, porque no había dónde dejarla: despejabas, te ibas al de al lado
    /// y al volver estaba todo otra vez encima. Ahora cada escritorio recuerda
    /// que está despejado hasta que se pide lo contrario.
    escondidas: HashMap<String, Vec<Vec<Apartada>>>,
    /// La animación de despejar o devolver, si hay una en marcha.
    despeje: Option<Despeje>,
    /// Los cambios que se están viendo ahora mismo, como mucho uno por salida.
    deslizando: HashMap<String, Deslizamiento>,
}

impl Escritorios {
    pub fn new(cuantos: usize, mut nombres: Vec<String>) -> Self {
        let cuantos = cuantos.clamp(1, MAXIMO);
        nombres.truncate(cuantos);
        while nombres.len() < cuantos {
            nombres.push(format!("Escritorio {}", nombres.len() + 1));
        }
        // Con uno solo no hay a dónde ir y el gesto de cambiar de escritorio no
        // puede hacer nada: se dice al arrancar para no diagnosticarlo a ciegas.
        tracing::info!(cuantos, "escritorios virtuales");
        Self {
            activo: HashMap::new(),
            cuantos,
            nombres,
            guardadas: HashMap::new(),
            escondidas: HashMap::new(),
            despeje: None,
            deslizando: HashMap::new(),
        }
    }

    /// El escritorio activo de una salida. Una que nunca se haya movido está en
    /// el primero.
    pub fn activo_en(&self, salida: &str) -> usize {
        self.activo
            .get(salida)
            .copied()
            .unwrap_or(0)
            .min(self.cuantos - 1)
    }

    /// Las guardadas de una salida, creándole su hueco si es la primera vez.
    fn guardadas_de(&mut self, salida: &str) -> &mut Vec<Vec<Apartada>> {
        let cuantos = self.cuantos;
        self.guardadas
            .entry(salida.to_string())
            .or_insert_with(|| (0..cuantos).map(|_| Vec::new()).collect())
    }

    /// Las escondidas de una salida, igual que [`Self::guardadas_de`].
    fn escondidas_de(&mut self, salida: &str) -> &mut Vec<Vec<Apartada>> {
        let cuantos = self.cuantos;
        self.escondidas
            .entry(salida.to_string())
            .or_insert_with(|| (0..cuantos).map(|_| Vec::new()).collect())
    }

    pub fn cuantos(&self) -> usize {
        self.cuantos
    }

    pub fn nombres(&self) -> &[String] {
        &self.nombres
    }

    /// ¿Hay algún cambio de escritorio a la vista ahora mismo, en cualquier
    /// salida? Es lo que impide que el bucle se duerma a mitad de la
    /// transición, así que basta con que quede uno.
    pub fn deslizando(&self) -> bool {
        !self.deslizando.is_empty() || self.despeje.is_some()
    }

    /// Cuánto se lleva del cambio de esa salida, de 0 a 1, **sin curva**: es
    /// tiempo, no recorrido. Quien anime le aplica la suya.
    fn fraccion(&self, salida: &str) -> Option<f32> {
        let d = self.deslizando.get(salida)?;
        Some(bookos_shell::tema::fraccion(
            d.desde.elapsed(),
            DESLIZAMIENTO,
        ))
    }

    /// Dónde va cada uno de los dos fondos mientras dura el cambio.
    ///
    /// `None` fuera de la transición: entonces el fondo se dibuja donde
    /// siempre y esto no cuesta nada.
    ///
    /// El fondo es **el mismo** en todos los escritorios, así que si se queda
    /// quieto el cambio no se ve en un escritorio vacío: no hay ventanas que
    /// deslizar y la única pista es el punto del panel. Aquí se desliza con
    /// ellas, a la misma velocidad y en la misma dirección: la pantalla entera
    /// —fondo y ventanas— es una sola tira que se corre de lado.
    ///
    /// Por eso son dos y no uno: el que se va deja el borde al descubierto, y
    /// lo que hay detrás es el del escritorio al que se llega, pegado a él.
    /// Devuelve `(x del saliente, x del entrante)` en lógicos.
    pub fn tira_fondo(&self, salida: &str) -> Option<(i32, i32)> {
        let d = self.deslizando.get(salida)?;
        let avance = bookos_shell::tema::C_ENTRADA.eval(self.fraccion(salida)?) as f64;
        // Las mismas cuentas que mueven las ventanas en `animar`: el fondo no
        // puede ir con su propia velocidad o la tira se rompería por la mitad.
        let recorrido = (d.ancho * d.direccion * avance) as i32;
        let entrante = (d.ancho * d.direccion) as i32 - recorrido;
        Some((-recorrido, entrante))
    }

    /// ¿Se está yendo esta ventana por un «mostrar escritorio» en marcha?
    pub fn se_va(&self, window: &Window) -> bool {
        self.despeje
            .as_ref()
            .is_some_and(|d| d.ida && d.ventanas.iter().any(|(w, _, _)| w == window))
    }

    /// ¿Hay algo apartado por «mostrar escritorio» en lo que se ve ahora, en
    /// alguna pantalla?
    pub fn despejado(&self) -> bool {
        self.escondidas_activas().next().is_some()
    }

    /// Las apartadas por «mostrar escritorio» del escritorio activo de cada
    /// salida: las que el gesto contrario devolvería.
    pub fn escondidas_activas(&self) -> impl Iterator<Item = &Window> {
        self.escondidas
            .iter()
            .filter_map(|(salida, listas)| listas.get(self.activo_en(salida)))
            .flatten()
            .map(|(w, _)| w)
    }

    /// Las ventanas que no están en el `Space` pero siguen vivas: las de los
    /// otros escritorios y las que apartó «mostrar escritorio».
    pub fn apartadas(&self) -> impl Iterator<Item = &Window> {
        self.escondidas
            .values()
            .chain(self.guardadas.values())
            .flatten()
            .flatten()
            .map(|(w, _)| w)
    }
}

/// Instantánea de las ventanas de todos los escritorios **de una salida**, para
/// la vista general. El activo sale del `Space`; los demás ya están guardados
/// aquí.
///
/// Toma la salida y no el escritorio entero porque los escritorios son por
/// salida: enseñar los cuatro con las ventanas de los dos monitores mezcladas
/// no describiría ningún estado que exista.
pub fn ventanas_para_vista(state: &BookosComp, salida: &str) -> Vec<Vec<Apartada>> {
    let activo = state.escritorios.activo_en(salida);
    let guardadas = state.escritorios.guardadas.get(salida);
    (0..state.escritorios.cuantos)
        .map(|i| {
            if i == activo {
                ventanas_de(state, salida)
            } else {
                guardadas
                    .and_then(|g| g.get(i))
                    .cloned()
                    .unwrap_or_default()
            }
        })
        .collect()
}

/// La salida cuyos escritorios enseña la vista general: la que tiene el puntero
/// encima, que es sobre la que van a actuar el clic y el gesto.
pub fn salida_para_vista(state: &BookosComp) -> String {
    state
        .salida_del_puntero()
        .map(|o| o.name())
        .unwrap_or_default()
}

/// Añade un escritorio al final. La vista actual no cambia.
pub fn crear(state: &mut BookosComp) -> bool {
    terminar_todos(state);
    if state.escritorios.cuantos >= MAXIMO {
        return false;
    }
    // Un hueco más en cada salida: el número de escritorios es global.
    for lista in state
        .escritorios
        .guardadas
        .values_mut()
        .chain(state.escritorios.escondidas.values_mut())
    {
        lista.push(Vec::new());
    }
    state.escritorios.cuantos += 1;
    state
        .escritorios
        .nombres
        .push(format!("Escritorio {}", state.escritorios.cuantos));
    avisar_al_panel(state);
    state.needs_redraw = true;
    true
}

/// Cambia el nombre visible, limpiando los separadores del formato de config.
pub fn renombrar(state: &mut BookosComp, indice: usize, nombre: &str) -> bool {
    let Some(actual) = state.escritorios.nombres.get_mut(indice) else {
        return false;
    };
    let limpio: String = nombre
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '\n' | '\r') && !c.is_control())
        .take(32)
        .collect();
    if limpio.is_empty() || *actual == limpio {
        return false;
    }
    *actual = limpio;
    state.needs_redraw = true;
    true
}

/// Lleva los escritorios a lo que dice `panel.conf`: cuántos hay y cómo se
/// llaman. Es lo que hace en vivo un cambio desde Ajustes; antes solo se leía
/// al arrancar, y la vista de Meta+W seguía con el número viejo hasta volver a
/// iniciar sesión.
///
/// Por las mismas `crear` y `eliminar` que usa la vista, así que quitar
/// escritorios lleva sus ventanas al vecino en vez de cerrarlas. Se quitan
/// por el final, que es lo que se espera al bajar el número. Devuelve si
/// cambió algo.
pub fn ajustar(state: &mut BookosComp, cuantos: usize, nombres: &[String]) -> bool {
    let cuantos = cuantos.clamp(1, MAXIMO);
    let mut cambio = false;
    while state.escritorios.cuantos < cuantos && crear(state) {
        cambio = true;
    }
    while state.escritorios.cuantos > cuantos && eliminar(state, state.escritorios.cuantos - 1) {
        cambio = true;
    }
    for (i, nombre) in nombres.iter().enumerate().take(cuantos) {
        cambio |= renombrar(state, i, nombre);
    }
    cambio
}

/// Elimina un escritorio, pero nunca el último. Sus ventanas se trasladan al
/// vecino más próximo para que borrar organización nunca cierre trabajo.
///
/// El escritorio se quita **en todas las salidas**: el número de escritorios es
/// un ajuste del usuario y es el mismo en todas. Lo que se hace por separado es
/// el reparto de las ventanas y el índice activo, porque cada monitor puede
/// estar en un escritorio distinto y el borrado le desplaza el suyo de otra
/// forma.
pub fn eliminar(state: &mut BookosComp, indice: usize) -> bool {
    terminar_todos(state);
    soltar_escondidas(state);
    purgar(state);
    if state.escritorios.cuantos <= 1 || indice >= state.escritorios.cuantos {
        return false;
    }

    // Todas las salidas con estado: las enchufadas ahora y las que tengan
    // ventanas guardadas de un monitor que se desenchufó y volverá.
    let mut salidas: Vec<String> = state.space.outputs().map(|o| o.name()).collect();
    for nombre in state.escritorios.guardadas.keys() {
        if !salidas.contains(nombre) {
            salidas.push(nombre.clone());
        }
    }

    for salida in salidas {
        let activo_anterior = state.escritorios.activo_en(&salida);
        let borradas = if indice == activo_anterior {
            let suyas = ventanas_de(state, &salida);
            for (window, _) in &suyas {
                state.space.unmap_elem(window);
            }
            suyas
        } else {
            std::mem::take(&mut state.escritorios.guardadas_de(&salida)[indice])
        };

        state.escritorios.guardadas_de(&salida).remove(indice);

        // Si se borra el activo se muestra el que ocupó su sitio, o el anterior
        // si era el último. Si se borra otro, el índice activo solo se desplaza.
        let nuevo_activo = if indice == activo_anterior {
            indice.min(state.escritorios.cuantos - 2)
        } else if indice < activo_anterior {
            activo_anterior - 1
        } else {
            activo_anterior
        };
        state
            .escritorios
            .activo
            .insert(salida.clone(), nuevo_activo);

        let destino = if indice == activo_anterior {
            nuevo_activo
        } else if indice == 0 {
            0
        } else {
            indice - 1
        };
        if destino == nuevo_activo {
            let mut a_mapear =
                std::mem::take(&mut state.escritorios.guardadas_de(&salida)[destino]);
            a_mapear.extend(borradas);
            for (window, posicion) in a_mapear {
                state.space.map_element(window, posicion, false);
            }
        } else {
            state.escritorios.guardadas_de(&salida)[destino].extend(borradas);
        }
    }

    state.escritorios.nombres.remove(indice);
    state.escritorios.cuantos -= 1;

    // El foco, a la última de la salida donde está el puntero: es la que el
    // usuario está mirando cuando confirma el borrado.
    let aqui = salida_para_vista(state);
    let ultima = state
        .space
        .elements()
        .filter(|&w| salida_de(state, w).as_deref() == Some(aqui.as_str()))
        .cloned()
        .next_back();
    if let Some(window) = ultima {
        state.enfocar(&window);
    }

    avisar_al_panel(state);
    state.needs_redraw = true;
    true
}

/// A qué escritorio lleva moverse `pasos` desde `activo`, con `cuantos` en total.
///
/// **No hay vuelta circular**: en el último, seguir a la derecha no lleva al
/// primero. Con un gesto de cuatro dedos el envolvimiento es desorientador —te
/// vas al otro extremo del escritorio sin querer— y no hay forma de saber por
/// el movimiento que has llegado al final. Quedarse quieto sí la hay.
pub fn destino(activo: usize, pasos: i32, cuantos: usize) -> usize {
    (activo as i32 + pasos).clamp(0, cuantos as i32 - 1) as usize
}

/// Cuánto dura el deslizamiento entre escritorios.
///
/// Es la duración de «transición de página completa» del sistema de diseño, con
/// su curva —sin rebote—: cambiar de escritorio **es** una transición de página,
/// y usar la misma tabla que el launchpad es lo que hace que el escritorio se
/// mueva siempre igual. Nada de un valor propio aquí.
const DESLIZAMIENTO: std::time::Duration = bookos_shell::tema::D_PAGINA;

/// A qué salida pertenece una ventana: la que contiene su centro.
///
/// Por el centro y no por la esquina porque una ventana puede cruzar el borde
/// entre dos monitores, y en ese caso «está» en el que enseña más de ella. Sin
/// salida —ventana fuera de todo, que pasa mientras se recoloca— devuelve
/// `None` y quien llame decide.
pub fn salida_de(state: &BookosComp, window: &Window) -> Option<String> {
    let geo = state.space.element_geometry(window)?;
    let centro = (
        geo.loc.x as f64 + geo.size.w as f64 / 2.0,
        geo.loc.y as f64 + geo.size.h as f64 / 2.0,
    );
    state
        .space
        .outputs()
        .find(|o| {
            state
                .space
                .output_geometry(o)
                .is_some_and(|r| r.to_f64().contains(centro))
        })
        .map(|o| o.name())
}

/// Las ventanas del `Space` que están en esta salida, con su posición.
fn ventanas_de(state: &BookosComp, salida: &str) -> Vec<Apartada> {
    state
        .space
        .elements()
        .filter(|&w| salida_de(state, w).as_deref() == Some(salida))
        .cloned()
        .filter_map(|w| state.space.element_location(&w).map(|p| (w, p)))
        .collect()
}

/// Cambia el escritorio **de una salida**. No hace nada si ya está en él.
///
/// Solo se lleva las ventanas de esa salida: las del otro monitor se quedan
/// donde están. Y desliza el ancho de **esa** pantalla, no el de la que tenga
/// el puntero encima: con dos monitores de distinto ancho, usar el ajeno dejaba
/// las ventanas a medio camino durante la transición.
pub fn cambiar_a(state: &mut BookosComp, salida: &Output, n: usize) {
    let nombre = salida.name();
    let anterior = state.escritorios.activo_en(&nombre);
    if n >= state.escritorios.cuantos || n == anterior {
        tracing::debug!(
            pedido = n,
            activo = anterior,
            cuantos = state.escritorios.cuantos,
            salida = %nombre,
            "cambio de escritorio descartado"
        );
        return;
    }
    // Lo despejado se queda despejado: sus ventanas siguen en `escondidas`, a
    // nombre de este escritorio, y volver a él no las trae. Antes se devolvían
    // aquí y al cambiar de escritorio reaparecía todo lo que se había
    // apartado. Lo que sí hay que hacer es acabar la animación, o unas
    // ventanas a medio camino se irían como salientes del escritorio.
    terminar_despeje(state);
    purgar(state);
    // Y si ya había un deslizamiento a medias **en esta salida**, se termina de
    // golpe antes de empezar otro: dos transiciones a la vez dejarían ventanas
    // de tres escritorios en pantalla. Las de las demás salidas siguen su
    // camino, que es justo lo que las hace independientes.
    terminar_deslizamiento(state, &nombre);

    let direccion = if n > anterior { 1.0 } else { -1.0 };
    let ancho = state
        .space
        .output_geometry(salida)
        .map(|r| r.size.w as f64)
        .unwrap_or_else(|| state.work_area().size.w as f64);

    let salientes = ventanas_de(state, &nombre);
    let entrantes = std::mem::take(&mut state.escritorios.guardadas_de(&nombre)[n]);
    state.escritorios.guardadas_de(&nombre)[anterior] = salientes.clone();
    state.escritorios.activo.insert(nombre.clone(), n);

    // Las que entran se mapean **fuera de la pantalla**, del lado del que
    // vienen, y desde ahí se deslizan hasta su sitio.
    for (window, destino) in &entrantes {
        let inicio = (destino.x + (ancho * direccion) as i32, destino.y);
        state.space.map_element(window.clone(), inicio, false);
    }

    state.escritorios.deslizando.insert(
        nombre.clone(),
        Deslizamiento {
            desde: std::time::Instant::now(),
            direccion,
            ancho,
            salientes,
            entrantes,
        },
    );

    tracing::debug!(de = anterior, a = n, salida = %nombre, "escritorio");
    avisar_al_panel(state);
    // Y el aviso con el nombre, que es lo único que se ve cuando el escritorio
    // al que vas está vacío: sin ventanas no hay nada que deslizar.
    let nombre_escritorio = state
        .escritorios
        .nombres
        .get(n)
        .cloned()
        .unwrap_or_else(|| format!("Escritorio {}", n + 1));
    // Con más de un monitor, el aviso dice también en cuál ha pasado: el OSD se
    // dibuja siempre en la principal, así que sin esto un cambio en la
    // secundaria parecería haber ocurrido aquí.
    let texto = if state.space.outputs().count() > 1 {
        format!("{nombre_escritorio} · {nombre}")
    } else {
        nombre_escritorio
    };
    let icono = format!("escritorio-{}", n + 1);
    if let Some(shell) = state.shell.as_mut() {
        shell.mostrar_osd(&icono, None, Some(texto));
    }
    state.needs_redraw = true;
}

/// Cambia el escritorio de la salida **bajo el puntero**.
///
/// Es la puerta que usan los atajos y los gestos, y por eso la decisión de «en
/// qué monitor» está aquí y en un solo sitio: donde está el ratón es donde el
/// usuario está mirando, y es lo mismo que ya deciden `maximizar` y `encajar`
/// a través de `ventanas::pantalla`.
pub fn cambiar_aqui(state: &mut BookosComp, n: usize) {
    let Some(salida) = state.salida_del_puntero().cloned() else {
        return;
    };
    cambiar_a(state, &salida, n);
}

/// El escritorio activo de la salida bajo el puntero.
pub fn activo_aqui(state: &BookosComp) -> usize {
    state.escritorios.activo_en(&salida_para_vista(state))
}

/// Le dice al indicador del panel dónde está la sesión.
///
/// El shell no puede saberlo por su cuenta —los escritorios los gobierna el
/// compositor— y se le cuenta **al cambiar**, no en cada frame: el widget
/// compara y calla si no ha cambiado nada, pero preguntárselo sesenta veces por
/// segundo sería trabajo por nada.
///
/// Con escritorios por salida hay que elegir cuál enseña, porque el panel es
/// uno solo: enseña el de la salida **bajo el puntero**. Es la que el usuario
/// acaba de mover, así que la respuesta cae donde está mirando; con una sola
/// pantalla es exactamente lo de siempre.
pub fn avisar_al_panel(state: &mut BookosComp) {
    let salida = state
        .salida_del_puntero()
        .map(|o| o.name())
        .unwrap_or_default();
    let (activo, cuantos) = (
        state.escritorios.activo_en(&salida),
        state.escritorios.cuantos,
    );
    if let Some(shell) = state.shell.as_mut() {
        shell.escritorios(activo, cuantos);
    }
}

/// Mueve las ventanas al punto que les toca del deslizamiento.
///
/// Lo llama el compositor antes de componer cada frame, como el resto de
/// animaciones. Devuelve `true` mientras siga habiendo algo que mover, que es lo
/// que impide que el bucle se duerma a mitad de la transición.
pub fn animar(state: &mut BookosComp) -> bool {
    let despejando = animar_despeje(state);
    if state.escritorios.deslizando.is_empty() {
        return despejando;
    }
    // Se recogen los nombres antes de tocar nada: `terminar_deslizamiento`
    // muta el mapa, y hay que poder terminar uno mientras los demás siguen.
    let salidas: Vec<String> = state.escritorios.deslizando.keys().cloned().collect();
    let mut vivo = false;
    for salida in salidas {
        let Some(d) = state.escritorios.deslizando.get(&salida) else {
            continue;
        };
        let t = bookos_shell::tema::fraccion(d.desde.elapsed(), DESLIZAMIENTO);
        if t >= 1.0 {
            terminar_deslizamiento(state, &salida);
            continue;
        }
        let avance = bookos_shell::tema::C_ENTRADA.eval(t) as f64;
        // Lo que se ha recorrido de pantalla en este instante. Las que se van lo
        // restan y las que entran lo suman: se mueven juntas, como una sola tira.
        let recorrido = (d.ancho * d.direccion * avance) as i32;
        let desplazada = (d.ancho * d.direccion) as i32 - recorrido;
        let salientes: Vec<Apartada> = d.salientes.clone();
        let entrantes: Vec<Apartada> = d.entrantes.clone();
        for (window, origen) in salientes {
            state
                .space
                .map_element(window, (origen.x - recorrido, origen.y), false);
        }
        for (window, destino) in entrantes {
            state
                .space
                .map_element(window, (destino.x + desplazada, destino.y), false);
        }
        vivo = true;
    }
    state.needs_redraw = true;
    vivo || despejando
}

/// Deja las ventanas de esa salida donde tienen que acabar y suelta su
/// animación.
///
/// Es idempotente y se llama tanto al terminar el tiempo como al empezar otro
/// cambio: el estado final no puede depender de que la animación se haya visto
/// entera, porque un cambio rápido de dos escritorios la interrumpe siempre.
fn terminar_deslizamiento(state: &mut BookosComp, salida: &str) {
    let Some(d) = state.escritorios.deslizando.remove(salida) else {
        return;
    };
    for (window, _) in &d.salientes {
        state.space.unmap_elem(window);
    }
    for (window, destino) in &d.entrantes {
        state.space.map_element(window.clone(), *destino, false);
    }
    // El foco va a la última **de esta salida** —la que estaba arriba del todo
    // cuando se salió— y, si no hay ninguna, se retira: dejárselo a una ventana
    // que ya no se ve significa escribir a ciegas en ella. Antes se cogía la
    // última de todo el `Space`, que con dos monitores podía estar en el otro.
    let ultima = state
        .space
        .elements()
        .filter(|&w| salida_de(state, w).as_deref() == Some(salida))
        .cloned()
        .next_back();
    match ultima {
        Some(window) => state.enfocar(&window),
        None => {
            if let Some(kbd) = state.seat.get_keyboard() {
                kbd.set_focus(state, None, smithay::utils::SERIAL_COUNTER.next_serial());
            }
        }
    }
    // El dock enseña las aplicaciones del escritorio en el que estás, y el
    // `Space` acaba de cambiar de juego de ventanas. Sin esto se quedaba con la
    // lista del escritorio anterior: un escritorio vacío seguía enseñando con
    // su punto una aplicación abierta en el de al lado.
    state.actualizar_dock();
    // Y las barras, por lo mismo: pasar de un escritorio con una ventana
    // pequeña a otro con una maximizada dejaba el panel a la vista encima de
    // ella, o escondido sobre un escritorio vacío.
    state.revisar_barras();
    state.needs_redraw = true;
}

/// Termina de golpe los cambios en marcha en **todas** las salidas. Lo usan las
/// operaciones que reorganizan los escritorios enteros (crear, eliminar): ahí
/// dejar una transición a medias dejaría ventanas mapeadas fuera de sitio.
pub fn terminar_todos(state: &mut BookosComp) {
    let salidas: Vec<String> = state.escritorios.deslizando.keys().cloned().collect();
    for salida in salidas {
        terminar_deslizamiento(state, &salida);
    }
    terminar_despeje(state);
}

/// Envía una ventana al escritorio de su monitor sin cambiar la vista.
pub fn mover_ventana(state: &mut BookosComp, window: &Window, destino: usize) {
    let familia = crate::multitarea::familia(state, window);
    for w in familia {
        mover_ventana_una(state, &w, destino);
    }
}

fn mover_ventana_una(state: &mut BookosComp, window: &Window, destino: usize) {
    if state.minimizando.iter().any(|(w, _)| w == window) {
        return;
    }
    if destino >= state.escritorios.cuantos() {
        return;
    }
    terminar_todos(state);
    let Some(salida) = salida_de(state, window) else {
        return;
    };
    if destino == state.escritorios.activo_en(&salida) {
        return;
    }
    let Some(posicion) = state.space.element_location(window) else {
        return;
    };
    let tenia_foco = state.ventana_con_foco().as_ref() == Some(window);
    state.space.unmap_elem(window);
    state.escritorios.guardadas_de(&salida)[destino].push((window.clone(), posicion));
    if window.set_activated(false)
        && let Some(toplevel) = window.toplevel()
    {
        toplevel.send_pending_configure();
    }
    if tenia_foco {
        let siguiente = state
            .space
            .elements()
            .rev()
            .find(|w| salida_de(state, w).as_deref() == Some(&salida))
            .cloned();
        if let Some(w) = siguiente {
            state.enfocar(&w);
        } else if let Some(kbd) = state.seat.get_keyboard() {
            kbd.set_focus(state, None, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }
    state.revisar_barras();
    // Se ha ido del escritorio que se ve: su punto en el dock también.
    state.actualizar_dock();
    state.needs_redraw = true;
}

/// Aparta todas las ventanas para enseñar el escritorio.
///
/// Es el mismo mecanismo que el cambio de escritorio —desmapear guardando la
/// posición—, y por eso no cuesta casi nada: la alternativa sería minimizar de
/// verdad cada ventana, que exige estado por ventana y un sitio del que
/// restaurarla.
///
/// Va **separado** de [`recuperar`] y no en un `alternar` porque el gesto es
/// direccional: cuatro dedos hacia abajo despejan y hacia arriba devuelven. Con
/// un alternar, bajar dos veces traía las ventanas de vuelta, que es lo
/// contrario de lo que pide la mano.
///
/// Con el escritorio ya despejado no se ignora: lo que se haya abierto después
/// también se aparta, y vuelve junto con lo demás.
pub fn despejar(state: &mut BookosComp) {
    terminar_despeje(state);
    let salidas: Vec<(String, Rectangle<i32, Logical>)> = state
        .space
        .outputs()
        .filter_map(|o| state.space.output_geometry(o).map(|g| (o.name(), g)))
        .collect();
    let mut viaje = Vec::new();
    for (salida, pantalla) in &salidas {
        let suyas = ventanas_de(state, salida);
        viaje.extend(
            suyas
                .iter()
                .map(|(w, sitio)| (w.clone(), *sitio, fuera_de_pantalla(w, *sitio, *pantalla))),
        );
        let activo = state.escritorios.activo_en(salida);
        state.escritorios.escondidas_de(salida)[activo].extend(suyas);
    }
    if viaje.is_empty() {
        return;
    }
    state.escritorios.despeje = Some(Despeje {
        desde: std::time::Instant::now(),
        ida: true,
        ventanas: viaje,
    });
    // Las barras que esquivan ventanas no deben esperar a que acabe el viaje
    // para volver: las que se van ya no cuentan (ver `revisar_barras`).
    state.revisar_barras();
    // El teclado no puede quedarse en una ventana que ya no se ve: lo que se
    // tecleara iría a ciegas a ella.
    state.desenfocar();
    state.needs_redraw = true;
}

/// Alterna entre despejado y no. Lo usa el **atajo de teclado**, que es una sola
/// tecla y no tiene dos direcciones que ofrecer.
pub fn alternar_despejado(state: &mut BookosComp) {
    if state.escritorios.despejado() {
        recuperar(state);
    } else {
        despejar(state);
    }
}

/// Devuelve las ventanas apartadas por [`despejar`] en el escritorio que se ve
/// en cada pantalla. Las de los otros escritorios siguen apartadas: volverán
/// cuando se pida estando en ellos.
pub fn recuperar(state: &mut BookosComp) {
    terminar_despeje(state);
    let salidas: Vec<(String, Rectangle<i32, Logical>)> = state
        .space
        .outputs()
        .filter_map(|o| state.space.output_geometry(o).map(|g| (o.name(), g)))
        .collect();
    let mut viaje = Vec::new();
    for (salida, pantalla) in &salidas {
        let activo = state.escritorios.activo_en(salida);
        let Some(lista) = state
            .escritorios
            .escondidas
            .get_mut(salida)
            .and_then(|l| l.get_mut(activo))
        else {
            continue;
        };
        // Entran por donde se fueron: se mapean fuera y la animación las trae.
        for (window, sitio) in std::mem::take(lista) {
            let fuera = fuera_de_pantalla(&window, sitio, *pantalla);
            state.space.map_element(window.clone(), fuera, false);
            viaje.push((window, sitio, fuera));
        }
    }
    if viaje.is_empty() {
        return;
    }
    if let Some(window) = state.space.elements().last().cloned() {
        state.enfocar(&window);
    }
    state.escritorios.despeje = Some(Despeje {
        desde: std::time::Instant::now(),
        ida: false,
        ventanas: viaje,
    });
    state.revisar_barras();
    state.needs_redraw = true;
}

/// Devuelve, si la aplicación tiene alguna ventana apartada por «mostrar el
/// escritorio» aquí, todo lo apartado, y le da el foco a la suya. Es lo que
/// espera quien pulsa su icono del dock: sin esto no la encontraba y lanzaba
/// otra instancia.
pub fn recuperar_app(state: &mut BookosComp, app_id: &str) -> bool {
    let de_la_app = |w: &Window| {
        crate::handlers::app_id(w).is_some_and(|suyo| bookos_shell::mismo_programa(app_id, &suyo))
    };
    let Some(window) = state
        .escritorios
        .escondidas_activas()
        .filter(|w| de_la_app(w))
        .last()
        .cloned()
    else {
        return false;
    };
    recuperar(state);
    state.enfocar(&window);
    true
}

/// Cuánto tardan las ventanas en irse a los bordes o en volver.
///
/// El de maximizar y encajar, con su curva: es una ventana moviéndose por la
/// pantalla, igual que aquellas, y debe sentirse igual de rápida.
const DESPEJE: std::time::Duration = bookos_shell::tema::D_VENTANA;

/// Lo que queda a la vista de cada ventana apartada, pegado al borde, como en
/// macOS: dice que las ventanas siguen ahí y es dónde pulsar para traerlas.
/// 24 px se ven sin quitarle escritorio a nadie y son un blanco cómodo.
const ASOMA: i32 = 24;

/// Adónde va una ventana al despejar: siempre hacia un lado, como en macOS.
/// Las de la mitad izquierda se van por la izquierda y el resto por la derecha.
///
/// Antes salían en línea recta desde el centro, y una ventana centrada o
/// maximizada —el caso normal— no tiene componente lateral: bajaba entera, que
/// se leía como un deslizamiento y no como apartar las ventanas. En horizontal
/// el movimiento es el mismo para todas y deja ver el escritorio de golpe.
fn fuera_de_pantalla(
    window: &Window,
    sitio: Point<i32, Logical>,
    pantalla: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let tam = window.geometry().size;
    // La barra de título la dibuja el compositor **encima** de la ventana: sin
    // contarla, una ventana que se fuera por abajo dejaría la barra asomando.
    let barra = if crate::decoracion::decorada(window) {
        crate::decoracion::ALTO
    } else {
        0
    };
    let caja = Rectangle::<i32, Logical>::new(
        (sitio.x, sitio.y - barra).into(),
        (tam.w, tam.h + barra).into(),
    );
    let (dx, dy) = salida_hacia_fuera(caja, pantalla);
    (sitio.x + dx, sitio.y + dy).into()
}

/// El desplazamiento horizontal que saca `caja` de `pantalla` por el lado más
/// cercano a su centro; a la derecha si cae justo en medio. Aparte de
/// [`fuera_de_pantalla`] para poder probarlo sin una `Window`.
fn salida_hacia_fuera(
    caja: Rectangle<i32, Logical>,
    pantalla: Rectangle<i32, Logical>,
) -> (i32, i32) {
    let centro = caja.loc.x + caja.size.w / 2;
    // `min`/`max`: una ventana que ya asomaba menos que eso no se mete hacia
    // dentro al despejar.
    if centro < pantalla.loc.x + pantalla.size.w / 2 {
        let der = caja.loc.x + caja.size.w;
        ((pantalla.loc.x + ASOMA - der).min(0), 0)
    } else {
        (
            (pantalla.loc.x + pantalla.size.w - ASOMA - caja.loc.x).max(0),
            0,
        )
    }
}

/// Las apartadas del escritorio activo de `salida`, en el borde donde asoman.
/// Vacío mientras viajan: entonces siguen en el `Space` y ya se dibujan.
pub fn asomadas(state: &BookosComp, salida: &str) -> Vec<(Window, Point<i32, Logical>)> {
    if state.escritorios.despeje.is_some() {
        return Vec::new();
    }
    let Some(pantalla) = state
        .space
        .outputs()
        .find(|o| o.name() == salida)
        .and_then(|o| state.space.output_geometry(o))
    else {
        return Vec::new();
    };
    state
        .escritorios
        .escondidas
        .get(salida)
        .and_then(|l| l.get(state.escritorios.activo_en(salida)))
        .into_iter()
        .flatten()
        .map(|(w, sitio)| (w.clone(), fuera_de_pantalla(w, *sitio, pantalla)))
        .collect()
}

/// ¿Cae el punto en el trozo que asoma de alguna apartada?
pub fn asomada_en(state: &BookosComp, punto: Point<f64, Logical>) -> bool {
    state.space.outputs().any(|o| {
        let Some(pantalla) = state.space.output_geometry(o) else {
            return false;
        };
        asomadas(state, &o.name()).iter().any(|(w, loc)| {
            Rectangle::new(*loc, w.geometry().size)
                .intersection(pantalla)
                .is_some_and(|r| r.to_f64().contains(punto))
        })
    })
}

/// Mueve las ventanas del despeje en marcha. `true` mientras le quede.
fn animar_despeje(state: &mut BookosComp) -> bool {
    let Some(d) = state.escritorios.despeje.as_ref() else {
        return false;
    };
    let t = match bookos_shell::tema::efectos_reducidos() {
        true => 1.0,
        false => bookos_shell::tema::fraccion(d.desde.elapsed(), DESPEJE),
    };
    if t >= 1.0 {
        terminar_despeje(state);
        return false;
    }
    let avance = bookos_shell::tema::C_VENTANA.eval(t) as f64;
    let peso = if d.ida { avance } else { 1.0 - avance };
    let posiciones: Vec<(Window, Point<i32, Logical>)> = d
        .ventanas
        .iter()
        .map(|(w, sitio, fuera)| {
            let x = sitio.x as f64 + (fuera.x - sitio.x) as f64 * peso;
            let y = sitio.y as f64 + (fuera.y - sitio.y) as f64 * peso;
            (w.clone(), (x.round() as i32, y.round() as i32).into())
        })
        .collect();
    mapear_en_su_orden(state, posiciones);
    state.needs_redraw = true;
    true
}

/// Deja el despeje en su estado final: las que se iban, fuera del `Space`; las
/// que volvían, en su sitio. Idempotente, como `terminar_deslizamiento`, y por
/// lo mismo: cualquier cosa que reorganice ventanas lo corta antes.
fn terminar_despeje(state: &mut BookosComp) {
    let Some(d) = state.escritorios.despeje.take() else {
        return;
    };
    if d.ida {
        for (window, _, _) in &d.ventanas {
            state.space.unmap_elem(window);
        }
    } else {
        let sitios = d
            .ventanas
            .into_iter()
            .map(|(w, sitio, _)| (w, sitio))
            .collect();
        mapear_en_su_orden(state, sitios);
    }
    state.revisar_barras();
    state.needs_redraw = true;
}

/// Recoloca ventanas sin desordenar la pila.
///
/// `map_element` sube la ventana arriba del todo. Llamándolo en el orden de la
/// lista, la ventana que el dock acababa de enfocar a mitad de la vuelta se
/// quedaba otra vez debajo en el fotograma siguiente; en el orden en que ya
/// están apiladas, cada una vuelve a quedar donde estaba.
fn mapear_en_su_orden(state: &mut BookosComp, mut ventanas: Vec<(Window, Point<i32, Logical>)>) {
    let pila: Vec<Window> = state.space.elements().cloned().collect();
    ventanas.sort_by_key(|(w, _)| pila.iter().position(|p| p == w));
    for (window, punto) in ventanas {
        state.space.map_element(window, punto, false);
    }
}

/// Devuelve a su escritorio, sin animación, todo lo apartado por «mostrar el
/// escritorio» en cualquiera de ellos. Lo usa eliminar un escritorio: los
/// índices se desplazan, y unas escondidas con el índice viejo acabarían en el
/// escritorio equivocado.
fn soltar_escondidas(state: &mut BookosComp) {
    terminar_despeje(state);
    let vivas: Vec<String> = state.space.outputs().map(|o| o.name()).collect();
    for (salida, listas) in std::mem::take(&mut state.escritorios.escondidas) {
        let activo = state.escritorios.activo_en(&salida);
        for (i, lista) in listas.into_iter().enumerate() {
            // Las del activo de una pantalla enchufada vuelven al `Space`; el
            // resto, a las guardadas de su escritorio, de donde las sacará un
            // cambio de escritorio o `adoptar_huerfanas` si la pantalla ya no
            // está.
            if i == activo && vivas.contains(&salida) {
                for (window, sitio) in lista {
                    state.space.map_element(window, sitio, false);
                }
            } else if let Some(hueco) = state.escritorios.guardadas_de(&salida).get_mut(i) {
                hueco.extend(lista);
            }
        }
    }
}

/// Adopta las ventanas guardadas de las salidas que ya no están.
///
/// Al desenchufar un monitor, sus ventanas del escritorio activo las devuelve
/// el `Space` —`unmap_output` no las pierde—, pero las de sus **otros**
/// escritorios se quedaban aquí colgando: nadie las volvía a mapear nunca, así
/// que el trabajo que hubiera ahí desaparecía hasta volver a enchufar ese
/// monitor exacto. Ahora pasan al escritorio del mismo número de la salida que
/// quede, reencuadradas dentro de ella: cambiar de escritorio las devuelve.
///
/// Se guarda **el escritorio activo** de la salida que se fue, sin embargo: si
/// vuelve a enchufarse aparece donde estaba. Lo que no puede quedarse son las
/// ventanas, que son trabajo.
pub fn adoptar_huerfanas(state: &mut crate::state::BookosComp) {
    let vivas: Vec<String> = state.space.outputs().map(|o| o.name()).collect();
    if vivas.is_empty() {
        return;
    }
    // Lo apartado por «mostrar el escritorio» en la pantalla que se fue se
    // trata como el resto de sus ventanas: pasa a sus guardadas y se adopta
    // con ellas, visible.
    let escondidas_muertas: Vec<String> = state
        .escritorios
        .escondidas
        .keys()
        .filter(|n| !vivas.contains(n))
        .cloned()
        .collect();
    for muerta in escondidas_muertas {
        let Some(listas) = state.escritorios.escondidas.remove(&muerta) else {
            continue;
        };
        for (i, lista) in listas.into_iter().enumerate() {
            if let Some(hueco) = state.escritorios.guardadas_de(&muerta).get_mut(i) {
                hueco.extend(lista);
            }
        }
    }
    let muertas: Vec<String> = state
        .escritorios
        .guardadas
        .keys()
        .filter(|n| !vivas.contains(n))
        .cloned()
        .collect();
    // La que adopta: la principal, que es la que siempre existe.
    let destino = vivas
        .iter()
        .find(|n| {
            state
                .space
                .outputs()
                .any(|o| o.name() == **n && o.current_location() == (0, 0).into())
        })
        .cloned()
        .unwrap_or_else(|| vivas[0].clone());
    let area = state
        .space
        .outputs()
        .find(|o| o.name() == destino)
        .and_then(|o| state.space.output_geometry(o));

    for muerta in muertas {
        let Some(listas) = state.escritorios.guardadas.remove(&muerta) else {
            continue;
        };
        let vacia = listas.iter().all(|l| l.is_empty());
        for (i, lista) in listas.into_iter().enumerate() {
            if lista.is_empty() {
                continue;
            }
            // Reencuadrar: las coordenadas eran del monitor que ya no está y
            // pueden ser negativas o caer más allá del que queda. Se recorta la
            // esquina dentro del área de destino; el tamaño de la ventana lo
            // arregla el cliente en su siguiente configure.
            let recolocadas: Vec<Apartada> = lista
                .into_iter()
                .map(|(w, p)| match area {
                    Some(r) => {
                        let x = p.x.clamp(r.loc.x, r.loc.x + r.size.w - 1);
                        let y = p.y.clamp(r.loc.y, r.loc.y + r.size.h - 1);
                        (w, (x, y).into())
                    }
                    None => (w, p),
                })
                .collect();
            let cuantos = state.escritorios.cuantos;
            let destino_lista = state.escritorios.guardadas_de(&destino);
            if let Some(hueco) = destino_lista.get_mut(i.min(cuantos - 1)) {
                hueco.extend(recolocadas);
            }
        }
        if !vacia {
            tracing::info!(salida = %muerta, %destino, "ventanas adoptadas de una pantalla que se fue");
        }
    }
}

/// Suelta las ventanas que ya han muerto.
///
/// Una ventana puede cerrarse mientras su escritorio no está a la vista: nadie
/// la desmapea del `Space` —no está en él— y se quedaría en la lista de
/// guardadas para **reaparecer muerta** al volver a ese escritorio. Se purga por
/// `alive()` y no por identidad porque los dos caminos de cierre —`xdg` y
/// XWayland— avisan de formas distintas, y este filtro vale para los dos.
pub fn purgar(state: &mut BookosComp) {
    use smithay::utils::IsAlive;
    for lista in state
        .escritorios
        .guardadas
        .values_mut()
        .chain(state.escritorios.escondidas.values_mut())
        .flat_map(|por_escritorio| por_escritorio.iter_mut())
        .chain(std::iter::once(&mut state.minimizadas))
        .chain(std::iter::once(&mut state.minimizando))
    {
        lista.retain(|(w, _)| w.alive());
    }
    state.capturas.retain(|(w, _)| w.alive());
    if let Some(d) = state.escritorios.despeje.as_mut() {
        d.ventanas.retain(|(w, _, _)| w.alive());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Al despejar, cada ventana sale por el lado en que tiene su centro, dejando
    /// una tira a la vista,
    /// y la centrada o maximizada también se va de lado, no hacia abajo.
    #[test]
    fn despejar_saca_cada_ventana_por_su_lado() {
        let pantalla = Rectangle::<i32, Logical>::new((0, 0).into(), (1600, 1000).into());
        let fuera = |x, y, w, h| {
            let caja = Rectangle::<i32, Logical>::new((x, y).into(), (w, h).into());
            let (dx, dy) = salida_hacia_fuera(caja, pantalla);
            let movida = Rectangle::<i32, Logical>::new((x + dx, y + dy).into(), (w, h).into());
            assert_eq!(
                movida.intersection(pantalla).map(|r| r.size.w),
                Some(ASOMA),
                "la ventana {caja:?} asoma justo una tira en {movida:?}"
            );
            (dx, dy)
        };
        let (dx, dy) = fuera(50, 300, 400, 300);
        assert!(dx < 0, "la de la izquierda se va por la izquierda");
        assert_eq!(dy, 0);
        let (dx, _) = fuera(1100, 300, 400, 300);
        assert!(dx > 0, "y la de la derecha por la derecha");
        let (dx, dy) = fuera(0, 0, 1600, 1000);
        assert!(dx > 0 && dy == 0, "la maximizada no baja: se va de lado");
    }

    /// Moverse por los escritorios se para en los extremos en vez de dar la
    /// vuelta: con un gesto, aparecer al otro lado es desorientador.
    #[test]
    fn los_extremos_no_dan_la_vuelta() {
        assert_eq!(
            destino(0, -1, 4),
            0,
            "a la izquierda del primero, el primero"
        );
        assert_eq!(destino(0, 1, 4), 1);
        assert_eq!(destino(3, 1, 4), 3, "y del último, el último");
        assert_eq!(destino(1, -1, 4), 0);
    }

    /// Un salto largo tampoco se sale por arriba: los atajos directos
    /// (Meta+1..5) pasan por aquí y un número de más no puede indexar fuera.
    #[test]
    fn un_salto_largo_se_recorta() {
        assert_eq!(destino(0, 99, 4), 3);
        assert_eq!(destino(0, -99, 4), 0);
    }

    /// Con un solo escritorio —`escritorios = 1`— no hay a dónde ir: los gestos
    /// y los atajos siguen llegando y no pueden mover nada.
    #[test]
    fn con_uno_solo_no_se_mueve_nada() {
        assert_eq!(destino(0, 1, 1), 0);
        assert_eq!(destino(0, -1, 1), 0);
    }

    /// Un cambio de escritorio a `t` de su recorrido, sin ventanas: lo que hace
    /// falta para mirar el fondo.
    /// El nombre de la salida de mentira que usan los tests del fondo.
    const SALIDA: &str = "prueba";

    fn a_medias(direccion: f64, t: f32) -> Escritorios {
        let mut e = Escritorios::new(2, Vec::new());
        e.deslizando.insert(
            SALIDA.to_string(),
            Deslizamiento {
                desde: std::time::Instant::now() - DESLIZAMIENTO.mul_f32(t),
                direccion,
                ancho: 2000.0,
                salientes: Vec::new(),
                entrantes: Vec::new(),
            },
        );
        e
    }

    /// Cada salida lleva su escritorio, y una que nunca se ha movido está en el
    /// primero sin que nadie la haya dado de alta.
    #[test]
    fn cada_salida_lleva_su_escritorio() {
        let mut e = Escritorios::new(4, Vec::new());
        assert_eq!(
            e.activo_en("eDP-1"),
            0,
            "una salida nueva empieza en el primero"
        );
        assert_eq!(e.activo_en("HDMI-1"), 0);

        e.activo.insert("eDP-1".into(), 2);
        assert_eq!(e.activo_en("eDP-1"), 2);
        assert_eq!(
            e.activo_en("HDMI-1"),
            0,
            "mover un monitor no puede mover el de al lado"
        );
    }

    /// Un índice guardado que ya no existe —se borraron escritorios mientras
    /// ese monitor estaba desenchufado— no puede indexar fuera.
    #[test]
    fn un_activo_guardado_de_mas_se_recorta() {
        let mut e = Escritorios::new(2, Vec::new());
        e.activo.insert("HDMI-1".into(), 7);
        assert_eq!(e.activo_en("HDMI-1"), 1);
    }

    /// Dos cambios a la vez, uno en cada monitor, no se pisan: cada tira va con
    /// su ancho y su dirección. Antes esto no podía ni plantearse —había un
    /// solo `Deslizamiento`— y el ancho salía de la pantalla bajo el puntero,
    /// así que el monitor grande se quedaba a media pantalla.
    #[test]
    fn dos_salidas_deslizan_cada_una_lo_suyo() {
        let mut e = Escritorios::new(2, Vec::new());
        let ahora = std::time::Instant::now();
        e.deslizando.insert(
            "ancha".into(),
            Deslizamiento {
                desde: ahora,
                direccion: 1.0,
                ancho: 1920.0,
                salientes: Vec::new(),
                entrantes: Vec::new(),
            },
        );
        e.deslizando.insert(
            "estrecha".into(),
            Deslizamiento {
                desde: ahora,
                direccion: -1.0,
                ancho: 1280.0,
                salientes: Vec::new(),
                entrantes: Vec::new(),
            },
        );
        // Cada tira mide **su** ancho de pantalla, y hacia su lado. Se compara
        // el hueco entre los dos fondos y no una posición absoluta: la posición
        // depende de cuánto lleve la animación, o sea del reloj, y el hueco es
        // el invariante —un ancho de pantalla, siempre—.
        let (sale_ancha, entra_ancha) = e.tira_fondo("ancha").expect("hay cambio en la ancha");
        let (sale_est, entra_est) = e.tira_fondo("estrecha").expect("hay cambio en la estrecha");
        assert_eq!(entra_ancha - sale_ancha, 1920, "la ancha desliza lo suyo");
        assert_eq!(
            entra_est - sale_est,
            -1280,
            "y la estrecha lo suyo, al revés"
        );
        // Y una salida sin cambio no dibuja tira ninguna.
        assert_eq!(e.tira_fondo("quieta"), None);
        assert!(
            e.deslizando(),
            "con cambios en marcha el bucle no puede dormirse"
        );
    }

    /// Quieto no hay tira que dibujar. Es la comprobación de que esto no
    /// cuesta nada cuando no hay nada que animar: sin transición, el fondo se
    /// dibuja donde siempre y su damage sigue siendo ninguno.
    #[test]
    fn sin_cambio_no_hay_tira() {
        assert_eq!(Escritorios::new(2, Vec::new()).tira_fondo(SALIDA), None);
    }

    /// Los dos fondos van **pegados**, siempre y en las dos direcciones: uno
    /// empieza donde acaba el otro. Si esto se va, aparece una banda de nada
    /// entre los dos escritorios a mitad del deslizamiento.
    #[test]
    fn los_dos_fondos_van_pegados() {
        for direccion in [1.0, -1.0] {
            for paso in 0..=10 {
                let t = paso as f32 / 10.0;
                let (sale, entra) = a_medias(direccion, t)
                    .tira_fondo(SALIDA)
                    .expect("hay cambio");
                assert_eq!(
                    (entra - sale) as f64,
                    2000.0 * direccion,
                    "la tira se rompe en t={t}"
                );
            }
        }
    }

    /// Y la tira recorre una pantalla entera: empieza con el saliente en su
    /// sitio y acaba con el entrante en el suyo, que es donde se queda al
    /// terminar la animación.
    #[test]
    fn la_tira_recorre_una_pantalla() {
        // Los extremos se comprueban con holgura de un puñado de píxeles y no
        // con igualdad exacta: `a_medias` fija el instante de arranque y entre
        // eso y leer `tira_fondo` pasan unos microsegundos, así que en t=0 la
        // tira ya se ha movido un píxel. Lo que se comprueba es dónde empieza y
        // dónde acaba el recorrido, no el reloj de la máquina.
        let cerca = |vista: Option<(i32, i32)>, esperado: (i32, i32)| {
            let (s, e) = vista.expect("hay cambio");
            assert!(
                (s - esperado.0).abs() <= 4 && (e - esperado.1).abs() <= 4,
                "la tira está en {s},{e} y debería andar por {},{}",
                esperado.0,
                esperado.1
            );
        };
        cerca(a_medias(1.0, 0.0).tira_fondo(SALIDA), (0, 2000));
        cerca(a_medias(1.0, 1.0).tira_fondo(SALIDA), (-2000, 0));
        cerca(a_medias(-1.0, 0.0).tira_fondo(SALIDA), (0, -2000));
        cerca(a_medias(-1.0, 1.0).tira_fondo(SALIDA), (2000, 0));
        // Y a mitad de camino va por en medio, no en un extremo.
        let (sale, _) = a_medias(1.0, 0.5).tira_fondo(SALIDA).unwrap();
        assert!(
            sale < -100 && sale > -1900,
            "a mitad el fondo está en {sale}"
        );
    }
}

/// Dónde cae cada ventana dentro de las miniaturas de la vista de escritorios:
/// el escritorio, la ventana y su rectángulo, en lógicos de pantalla. Dentro de
/// cada escritorio van de abajo arriba, como en el `Space`.
///
/// Es la misma cuenta para pintarlas y para saber qué hay bajo un clic: si
/// fueran dos cuentas, arrastrar cogería una ventana distinta de la que se ve.
pub fn ventanas_en_vista(state: &BookosComp) -> Vec<(usize, Window, Rectangle<i32, Logical>)> {
    let huecos = state
        .shell
        .as_ref()
        .map(|s| s.escritorios_miniaturas())
        .unwrap_or_default();
    let salida = salida_para_vista(state);
    let (ancho, alto) = state.pantalla_logica();
    ventanas_para_vista(state, &salida)
        .into_iter()
        .zip(huecos)
        .enumerate()
        .flat_map(|(i, (ventanas, hueco))| {
            let factor = (hueco.size.w as f64 / ancho.max(1.0) as f64)
                .min(hueco.size.h as f64 / alto.max(1.0) as f64);
            ventanas
                .into_iter()
                .filter(|(window, _)| crate::multitarea::visible(state, window))
                .map(move |(window, posicion)| {
                    let geo = window.geometry();
                    let rect = Rectangle::new(
                        (
                            hueco.loc.x + (posicion.x as f64 * factor).round() as i32,
                            hueco.loc.y + (posicion.y as f64 * factor).round() as i32,
                        )
                            .into(),
                        (
                            (geo.size.w as f64 * factor).round().max(1.0) as i32,
                            (geo.size.h as f64 * factor).round().max(1.0) as i32,
                        )
                            .into(),
                    );
                    (i, window, rect)
                })
        })
        .collect()
}

/// Las ventanas del escritorio actual repartidas bajo la franja de la vista,
/// para verlas enteras y poder cogerlas: con la franja abierta encima, las
/// ventanas de verdad quedan tapadas por el cristal, y sin esto solo se
/// podían mover desde su miniatura, que es diminuta.
///
/// Una rejilla de filas iguales, con cada ventana a escala dentro de su celda y
/// **nunca más grande** que en pantalla: agrandar una ventana pequeña la haría
/// parecer otra cosa. Van en el orden del `Space`, de abajo arriba, para que la
/// que tenías delante no salte de sitio cada vez que se abre la vista.
pub fn exposicion(state: &BookosComp) -> Vec<(Window, Rectangle<i32, Logical>)> {
    // El aire entre ventanas y hasta los bordes, en lógicos.
    const AIRE: i32 = 32;
    let Some(arriba) = state
        .shell
        .as_ref()
        .and_then(|s| s.vista_escritorios_fondo())
    else {
        return Vec::new();
    };
    let salida = salida_para_vista(state);
    let Some(pantalla) = state
        .space
        .outputs()
        .find(|o| o.name() == salida)
        .and_then(|o| state.space.output_geometry(o))
    else {
        return Vec::new();
    };
    let ventanas: Vec<_> = ventanas_de(state, &salida)
        .into_iter()
        .filter(|(window, _)| crate::multitarea::visible(state, window))
        .collect();
    let n = ventanas.len() as i32;
    if n == 0 {
        return Vec::new();
    }
    let zona = Rectangle::<i32, Logical>::new(
        (pantalla.loc.x + AIRE, arriba + AIRE).into(),
        (
            pantalla.size.w - AIRE * 2,
            pantalla.loc.y + pantalla.size.h - arriba - AIRE * 2,
        )
            .into(),
    );
    if zona.size.w <= 0 || zona.size.h <= 0 {
        return Vec::new();
    }
    // Tantas columnas como haga falta para que las celdas salgan lo más
    // parecidas posible a la proporción de la pantalla.
    let columnas = (1..=n)
        .min_by(|a, b| {
            let desvio = |c: i32| {
                let filas = (n + c - 1) / c;
                let proporcion =
                    (zona.size.w as f64 / c as f64) / (zona.size.h as f64 / filas as f64);
                (proporcion - pantalla.size.w as f64 / pantalla.size.h as f64).abs()
            };
            desvio(*a).total_cmp(&desvio(*b))
        })
        .unwrap_or(1);
    let filas = (n + columnas - 1) / columnas;
    let (celda_w, celda_h) = (zona.size.w / columnas, zona.size.h / filas);
    ventanas
        .into_iter()
        .enumerate()
        .map(|(i, (window, _))| {
            let (fila, col) = (i as i32 / columnas, i as i32 % columnas);
            // La última fila, si va incompleta, se centra.
            let en_fila = if fila == filas - 1 {
                n - fila * columnas
            } else {
                columnas
            };
            let sangria = (columnas - en_fila) * celda_w / 2;
            let geo = window.geometry().size;
            let factor = ((celda_w - AIRE) as f64 / geo.w.max(1) as f64)
                .min((celda_h - AIRE) as f64 / geo.h.max(1) as f64)
                .min(1.0);
            let (w, h) = (
                ((geo.w as f64 * factor).round() as i32).max(1),
                ((geo.h as f64 * factor).round() as i32).max(1),
            );
            let x = zona.loc.x + sangria + col * celda_w + (celda_w - w) / 2;
            let y = zona.loc.y + fila * celda_h + (celda_h - h) / 2;
            (window, Rectangle::new((x, y).into(), (w, h).into()))
        })
        .collect()
}

/// El paso de las ventanas entre su sitio en el escritorio y su casilla en la
/// vista general. Sin esto saltaban de golpe al abrir y al cerrar, y no se
/// veía qué casilla era qué ventana.
#[derive(Default)]
pub enum TransicionVista {
    #[default]
    Cerrada,
    /// Guarda la rejilla del último fotograma: al cerrar, la franja ya no
    /// existe y sin ella no se sabría desde dónde vuelven.
    Abierta {
        desde: std::time::Instant,
        rejilla: Vec<(Window, Rectangle<i32, Logical>)>,
    },
    Cerrando {
        desde: std::time::Instant,
        rejilla: Vec<(Window, Rectangle<i32, Logical>)>,
    },
}

impl TransicionVista {
    /// Lo que dura el viaje: el de una tarjeta, que es lo que tarda en
    /// asentarse la franja con su muelle.
    const DURACION: std::time::Duration = bookos_shell::tema::D_TARJETA;

    pub fn animando(&self) -> bool {
        match self {
            Self::Cerrada => false,
            Self::Abierta { desde, .. } | Self::Cerrando { desde, .. } => {
                desde.elapsed() < Self::DURACION
            }
        }
    }
}

/// Las ventanas de la vista general en este fotograma, con el rectángulo por
/// el que van de camino. Vacío cuando ni está abierta ni se está cerrando.
pub fn expuestas_ahora(
    state: &mut BookosComp,
    abierta: bool,
) -> Vec<(Window, Rectangle<i32, Logical>)> {
    use bookos_shell::tema;

    let ahora = std::time::Instant::now();
    let estado = std::mem::take(&mut state.transicion_vista);
    let (siguiente, rejilla, hacia_rejilla, desde) = match (estado, abierta) {
        (TransicionVista::Abierta { desde, .. }, true) => (true, exposicion(state), true, desde),
        (_, true) => (true, exposicion(state), true, ahora),
        (TransicionVista::Abierta { rejilla, .. }, false) => (false, rejilla, false, ahora),
        (TransicionVista::Cerrando { desde, rejilla }, false)
            if desde.elapsed() < TransicionVista::DURACION =>
        {
            (false, rejilla, false, desde)
        }
        (_, false) => return Vec::new(),
    };
    let t = if tema::efectos_reducidos() {
        1.0
    } else {
        tema::C_SUAVE.eval(tema::avance(desde.elapsed(), TransicionVista::DURACION)) as f64
    };
    // Cuánto de la rejilla hay en este fotograma: sube al abrir y baja al
    // cerrar.
    let peso = if hacia_rejilla { t } else { 1.0 - t };
    let lista = rejilla
        .iter()
        .filter_map(|(w, casilla)| {
            // Sin sitio en el escritorio —se cerró la vista cambiando de
            // escritorio y la ventana ya no está mapeada— no hay adónde volver:
            // se queda en su casilla y desaparece con la vista.
            let origen = state.space.element_geometry(w)?;
            let mezcla = |a: i32, b: i32| (a as f64 + (b - a) as f64 * peso).round() as i32;
            Some((
                w.clone(),
                Rectangle::new(
                    (
                        mezcla(origen.loc.x, casilla.loc.x),
                        mezcla(origen.loc.y, casilla.loc.y),
                    )
                        .into(),
                    (
                        mezcla(origen.size.w, casilla.size.w).max(1),
                        mezcla(origen.size.h, casilla.size.h).max(1),
                    )
                        .into(),
                ),
            ))
        })
        .collect();
    state.transicion_vista = if siguiente {
        TransicionVista::Abierta { desde, rejilla }
    } else {
        TransicionVista::Cerrando { desde, rejilla }
    };
    lista
}

/// Qué ventana de la vista hay bajo un punto: primero las repartidas bajo la
/// franja, que son las grandes, y luego las de las miniaturas. Devuelve su
/// escritorio, la ventana y dónde se ve.
fn ventana_bajo(
    state: &BookosComp,
    punto: Point<f64, Logical>,
) -> Option<(usize, Window, Rectangle<i32, Logical>)> {
    let activo = state.escritorios.activo_en(&salida_para_vista(state));
    exposicion(state)
        .into_iter()
        .rev()
        .map(|(w, r)| (activo, w, r))
        .chain(ventanas_en_vista(state).into_iter().rev())
        .find(|(_, _, r)| r.to_f64().contains(punto))
}

/// Zona sensible del cierre, igual que en la barra de título (HIG §11.2).
pub const LADO_CERRAR: i32 = 28;

/// El cierre conserva su tamaño al reducir la ventana para que siga siendo
/// pulsable. Sobresale de la esquina para no solaparse con el cierre de la
/// barra de título, que también se ve dentro de la miniatura.
pub fn boton_cerrar(window: &Window, destino: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let arriba = if crate::decoracion::decorada(window) {
        let factor = destino.size.w as f64 / window.geometry().size.w.max(1) as f64;
        destino.loc.y - (crate::decoracion::ALTO as f64 * factor).round() as i32
    } else {
        destino.loc.y
    };
    let medio = LADO_CERRAR / 2;
    Rectangle::new(
        (destino.loc.x + destino.size.w - medio, arriba - medio).into(),
        (LADO_CERRAR, LADO_CERRAR).into(),
    )
}

/// La ventana de la vista cuyo ✕ se enseña: la que tiene el puntero encima, o
/// su ✕. Solo una: con todas a la vez la rejilla se
/// llenaba de botones de cerrar, que es lo último que se quiere pulsar sin
/// querer.
pub fn con_cierre_visible(
    expuestas: &[(Window, Rectangle<i32, Logical>)],
    puntero: Point<f64, Logical>,
) -> Option<(Window, Rectangle<i32, Logical>)> {
    expuestas.iter().rev().find_map(|(w, r)| {
        let boton = boton_cerrar(w, *r);
        (r.to_f64().contains(puntero) || boton.to_f64().contains(puntero))
            .then(|| (w.clone(), boton))
    })
}

/// El mismo glifo, tinta y placa de la barra de título (HIG §11.2).
pub fn imagen_cerrar(
    lado: i32,
    señalado: bool,
) -> Option<smithay::backend::renderer::element::memory::MemoryRenderBuffer> {
    let pixmap = pixmap_cerrar(lado, señalado)?;
    // tiny-skia da RGBA premultiplicado: en little-endian es Abgr8888.
    Some(
        smithay::backend::renderer::element::memory::MemoryRenderBuffer::from_slice(
            pixmap.data(),
            smithay::backend::allocator::Fourcc::Abgr8888,
            (lado, lado),
            1,
            smithay::utils::Transform::Normal,
            None,
        ),
    )
}

fn pixmap_cerrar(lado: i32, señalado: bool) -> Option<resvg::tiny_skia::Pixmap> {
    use bookos_shell::tema;
    let fondo = tema::rojo();
    let tinta = if señalado {
        tema::tinta_sobre(fondo)
    } else {
        tema::texto()
    };
    let rgb = |[r, g, b]: [f32; 3]| {
        format!(
            "rgb({},{},{})",
            (r * 255.0).round(),
            (g * 255.0).round(),
            (b * 255.0).round()
        )
    };
    let placa = if señalado {
        format!(
            r#"<rect width="28" height="28" rx="6" fill="{}" fill-opacity="0.95"/>"#,
            rgb([fondo.r, fondo.g, fondo.b])
        )
    } else {
        String::new()
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 28 28">{placa}<path transform="translate(6 6)" d="M4.6 4.6 L11.4 11.4 M11.4 4.6 L4.6 11.4" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
        rgb([tinta.r, tinta.g, tinta.b]),
        if señalado { 1.0 } else { 0.55 },
    );
    let arbol = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(lado as u32, lado as u32)?;
    let k = lado as f32 / LADO_CERRAR as f32;
    resvg::render(
        &arbol,
        resvg::tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    Some(pixmap)
}

/// Una ventana cogida de una miniatura de la vista de escritorios.
pub struct ArrastreVista {
    pub window: Window,
    origen: usize,
    pulsado: Point<f64, Logical>,
    /// Distancia del puntero a la esquina de la miniatura: la miniatura sigue
    /// al puntero sin saltar a ponerle la esquina debajo.
    agarre: Point<f64, Logical>,
    tamano: Size<i32, Logical>,
    /// Ya pasó del umbral: es un arrastre y no un clic.
    pub moviendo: bool,
}

impl ArrastreVista {
    /// El rectángulo de la miniatura que va con el puntero.
    pub fn rect(&self, puntero: Point<f64, Logical>) -> Rectangle<i32, Logical> {
        Rectangle::new((puntero - self.agarre).to_i32_round(), self.tamano)
    }
}

/// El botón izquierdo con la vista de escritorios abierta. `true` si era para
/// esto y no hay que pasárselo a nadie más.
///
/// Pulsar sobre una ventana de una miniatura la coge, pero todavía no decide
/// nada: la vista cambia de escritorio **al pulsar**, así que el compositor se
/// queda la pulsación hasta saber si es un clic o un arrastre. Al soltar sin
/// haberla movido es un clic, y hace lo de siempre —ir a ese escritorio— y
/// además trae delante esa ventana, que es la que se ha señalado.
pub fn boton_en_vista(state: &mut BookosComp, pulsado: bool) -> bool {
    if !pulsado && std::mem::take(&mut state.cierre_en_vista) {
        return true;
    }
    let puntero = state.pointer_location;
    if pulsado
        && state
            .shell
            .as_ref()
            .is_some_and(|s| s.sobre_actividad(puntero.x, puntero.y))
    {
        return false;
    }
    if pulsado
        && state
            .shell
            .as_ref()
            .is_some_and(|s| s.vista_escritorios_abierta())
        && let Some((window, boton)) =
            con_cierre_visible(&exposicion(state), state.pointer_location)
        && boton.to_f64().contains(state.pointer_location)
    {
        // Una petición, como Meta+Q: la aplicación puede preguntar si guardar.
        // Si cierra, la rejilla se recoloca sola en el siguiente fotograma.
        crate::keybinds::cerrar(&window);
        state.cierre_en_vista = true;
        state.needs_redraw = true;
        return true;
    }
    if pulsado {
        if !state
            .shell
            .as_ref()
            .is_some_and(|s| s.vista_escritorios_abierta())
        {
            return false;
        }
        let puntero = state.pointer_location;
        let Some((origen, window, rect)) = ventana_bajo(state, puntero) else {
            return false;
        };
        // Lo que se lleva el puntero no pasa del 60 % del ancho de una
        // miniatura: una ventana cogida de la rejilla de abajo es grande, y a
        // su tamaño taparía la franja entera; al ancho justo de la miniatura
        // tapaba el marco resaltado del escritorio sobre el que se va a
        // soltar, que es lo único que dice adónde va. El punto por donde se
        // agarró se conserva a escala.
        let ancho_maximo = state
            .shell
            .as_ref()
            .and_then(|s| {
                s.escritorios_miniaturas()
                    .first()
                    .map(|h| h.size.w as f64 * 0.6)
            })
            .unwrap_or(rect.size.w as f64);
        let reduccion = (ancho_maximo / rect.size.w.max(1) as f64).min(1.0);
        let agarre = puntero - rect.loc.to_f64();
        state.arrastre_vista = Some(ArrastreVista {
            window,
            origen,
            pulsado: puntero,
            agarre: (agarre.x * reduccion, agarre.y * reduccion).into(),
            tamano: (
                ((rect.size.w as f64 * reduccion).round() as i32).max(1),
                ((rect.size.h as f64 * reduccion).round() as i32).max(1),
            )
                .into(),
            moviendo: false,
        });
        return true;
    }
    let Some(arrastre) = state.arrastre_vista.take() else {
        return false;
    };
    state.needs_redraw = true;
    let puntero = state.pointer_location;
    if !arrastre.moviendo {
        // Lo mismo que un clic en la miniatura cuando le llega al shell: cerrar
        // la vista e ir a ese escritorio.
        crate::keybinds::hacer(state, bookos_shell::Accion::VistaEscritorios);
        crate::keybinds::hacer(state, bookos_shell::Accion::Escritorio(arrastre.origen));
        if state.space.elements().any(|w| *w == arrastre.window) {
            state.enfocar(&arrastre.window);
        }
        return true;
    }
    let destino = state
        .shell
        .as_ref()
        .map(|s| s.escritorios_miniaturas())
        .unwrap_or_default()
        .iter()
        .position(|h| h.to_f64().contains(puntero));
    if let Some(destino) = destino.filter(|d| *d != arrastre.origen) {
        trasladar(state, &arrastre.window, arrastre.origen, destino);
    }
    true
}

/// El puntero se mueve con una ventana cogida en la vista. `true` si hay que
/// repintar.
pub fn mover_en_vista(state: &mut BookosComp) -> bool {
    let puntero = state.pointer_location;
    let Some(arrastre) = state.arrastre_vista.as_mut() else {
        return false;
    };
    if !arrastre.moviendo {
        let d = puntero - arrastre.pulsado;
        // El mismo umbral que arrastrar un icono del launchpad: por debajo, el
        // temblor de un clic normal no se confunde con un arrastre.
        if (d.x.hypot(d.y) as f32) < bookos_shell::UMBRAL_ARRASTRE {
            return false;
        }
        arrastre.moviendo = true;
    }
    true
}

/// Lleva una ventana de un escritorio a otro de la salida que enseña la vista,
/// esté donde esté: en el activo —mapeada en el `Space`— o apartada en otro.
pub fn trasladar(state: &mut BookosComp, window: &Window, origen: usize, destino: usize) {
    let salida = salida_para_vista(state);
    let activo = state.escritorios.activo_en(&salida);
    if origen == activo {
        mover_ventana(state, window, destino);
        return;
    }
    let guardadas = state.escritorios.guardadas_de(&salida);
    let Some(i) = guardadas
        .get(origen)
        .and_then(|g| g.iter().position(|(w, _)| w == window))
    else {
        return;
    };
    let (window, posicion) = guardadas[origen].remove(i);
    if destino == activo {
        // Al escritorio que se está viendo: vuelve al `Space` donde estaba y
        // se trae delante, que es adonde se ha llevado.
        state.space.map_element(window.clone(), posicion, true);
        state.enfocar(&window);
        state.revisar_barras();
    } else if let Some(g) = state.escritorios.guardadas_de(&salida).get_mut(destino) {
        g.push((window, posicion));
    }
    state.actualizar_dock();
    state.needs_redraw = true;
}
