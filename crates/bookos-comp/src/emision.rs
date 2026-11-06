//! Lo que el compositor guarda de cada pantalla que se está compartiendo.
//!
//! El vídeo sale por [`crate::pw`], que vive en otro hilo. Aquí solo está la
//! parte que el bucle de frames necesita saber: qué salida va a qué sesión, a
//! qué ritmo como mucho, y de dónde salen los `Vec` en los que se copia.
//!
//! El puntero **siempre** sale en la imagen: `escena()` lo pinta como un
//! elemento más y no hay forma de componer la salida sin él sin dibujarla dos
//! veces. Por eso el portal anuncia solo el modo de cursor «incrustado».
//!
//! ## El ritmo lo marca el escritorio, no un temporizador
//!
//! No hay ningún reloj que pida fotogramas. Se emite en `servir_capturas`, o
//! sea justo después de que la salida se haya dibujado de verdad, y solo si ha
//! pasado el periodo del ritmo negociado. Con el escritorio quieto no sale
//! ninguno y el proceso no despierta, que es la regla de la casa; el consumidor
//! lo ve como un stream de ritmo variable, que es lo que se le anunció.
//!
//! ## Los buffers dan la vuelta
//!
//! Un fotograma de 1080p son 8,3 MB. Asignarlo y tirarlo treinta veces por
//! segundo es medio gigabyte por segundo de trabajo del asignador para nada,
//! así que el hilo de PipeWire devuelve el `Vec` vacío cuando termina con él y
//! aquí se reutiliza.

use std::collections::HashSet;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay::output::Output;
use smithay::utils::{Buffer as BufferCoord, Size};

use crate::pw::{Emisor, Orden};

/// Una pantalla compartida.
pub struct Emision {
    /// El identificador de la sesión del portal. Es el que conoce el hilo.
    pub sesion: u32,
    pub ruta: zbus::zvariant::OwnedObjectPath,
    pub output: Output,
    /// El tamaño que se negoció con PipeWire. Si la salida cambia de modo deja
    /// de cuadrar y la emisión se para: renegociar el formato es otra tanda, y
    /// mandar píxeles de un tamaño que no es enseña basura.
    pub tamano: Size<i32, BufferCoord>,
    pub periodo: Duration,
    pub ultimo: Instant,
}

/// Todas las emisiones vivas más el hilo que las sirve.
pub struct Emisiones {
    emisor: Option<Emisor>,
    /// Por donde vuelven los `Vec` que el hilo ya ha vaciado.
    reciclado: Option<mpsc::Receiver<(u32, Vec<u8>)>>,
    libres: Vec<Vec<u8>>,
    /// Sesiones que ya tienen un marco viajando hacia PipeWire. Esta barrera
    /// evita llenar un canal ilimitado si el consumidor se atasca.
    pendientes: HashSet<u32>,
    activas: Vec<Emision>,
}

impl std::fmt::Debug for Emisiones {
    // 0.1.0
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emisiones")
            .field("activas", &self.activas.len())
            .field("libres", &self.libres.len())
            .finish()
    }
}

impl Default for Emisiones {
    // 0.1.0
    fn default() -> Self {
        Self::new()
    }
}

impl Emisiones {
    /// No arranca el hilo de PipeWire: eso se hace en [`Self::hilo`] la primera
    /// vez que alguien quiere compartir. Una sesión que nunca comparte pantalla
    /// no paga una conexión a PipeWire ni un hilo.
    // 0.1.0
    pub fn new() -> Self {
        Self {
            emisor: None,
            reciclado: None,
            libres: Vec::new(),
            pendientes: HashSet::new(),
            activas: Vec::new(),
        }
    }

    /// El emisor, arrancando el hilo si hacía falta.
    // 0.1.0
    pub fn hilo(&mut self) -> Option<&Emisor> {
        if self.emisor.is_none()
            && let Some((emisor, reciclado)) = crate::pw::arrancar()
        {
            self.emisor = Some(emisor);
            self.reciclado = Some(reciclado);
        }
        self.emisor.as_ref()
    }

    // 0.1.0
    pub fn anadir(&mut self, emision: Emision) {
        self.activas.push(emision);
    }

    /// Para una sesión y le dice al hilo que suelte el nodo.
    // 0.1.0
    pub fn quitar(&mut self, sesion: u32) {
        self.activas.retain(|e| e.sesion != sesion);
        self.pendientes.remove(&sesion);
        if let Some(emisor) = self.emisor.as_ref() {
            emisor.enviar(Orden::Cerrar { sesion });
        }
    }

    // 0.1.0
    pub fn hay(&self) -> bool {
        !self.activas.is_empty()
    }

    /// Las emisiones de esta salida a las que ya les toca fotograma.
    // 0.1.0
    pub fn tocan(&mut self, output: &Output, ahora: Instant) -> Vec<usize> {
        self.recoger();
        self.activas
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.output == *output
                    && !self.pendientes.contains(&e.sesion)
                    && ahora.duration_since(e.ultimo) >= e.periodo
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Manda un fotograma ya en BGRx. `indice` sale de [`Self::tocan`].
    ///
    /// `invertida` sale de [`crate::captura::hay_que_voltear`], que mira el
    /// `Transform` de la salida. Antes salía de `TextureMapping::flipped`, que
    /// es una constante, y por eso quien miraba la pantalla compartida desde una
    /// sesión de verdad la veía espejada en vertical.
    // 0.1.0
    pub fn emitir(
        &mut self,
        indice: usize,
        pixeles: &[u8],
        tamano: Size<i32, BufferCoord>,
        invertida: bool,
        ahora: Instant,
    ) -> Option<zbus::zvariant::OwnedObjectPath> {
        let emision = self.activas.get(indice)?;
        let sesion = emision.sesion;
        if tamano != emision.tamano {
            // La salida ha cambiado de modo por debajo. Se para en vez de
            // mandar píxeles que no cuadran con el formato negociado.
            tracing::info!(sesion, "la salida cambió de tamaño; se corta la emisión");
            let ruta = emision.ruta.clone();
            self.quitar(sesion);
            return Some(ruta);
        }

        let stride = tamano.w.max(0) as usize * 4;
        let alto = tamano.h.max(0) as usize;
        let necesarios = stride * alto;
        if pixeles.len() < necesarios {
            tracing::warn!(
                sesion,
                "el readback trajo menos píxeles de los que mide la salida"
            );
            return None;
        }
        self.activas[indice].ultimo = ahora;

        self.recoger();
        let mut datos = self.libres.pop().unwrap_or_default();
        datos.clear();
        datos.reserve(necesarios);
        if invertida {
            for fila in (0..alto).rev() {
                datos.extend_from_slice(&pixeles[fila * stride..(fila + 1) * stride]);
            }
        } else {
            datos.extend_from_slice(&pixeles[..necesarios]);
        }

        if let Some(emisor) = self.emisor.as_ref() {
            self.pendientes.insert(sesion);
            emisor.enviar(Orden::Marco { sesion, datos });
        }
        None
    }

    /// Recupera los `Vec` que el hilo ya ha soltado.
    // 0.1.0
    fn recoger(&mut self) {
        let Some(reciclado) = self.reciclado.as_ref() else {
            return;
        };
        // Tres son los buffers que PipeWire tiene; más de eso en la reserva
        // sería memoria parada.
        while let Ok((sesion, v)) = reciclado.try_recv() {
            self.pendientes.remove(&sesion);
            if self.libres.len() < 3 {
                self.libres.push(v);
            }
        }
    }
}
