//! añadido en la 0.2.0 -- los guiones atraviesan input::handle, incluidos bloqueo, kiosko y métricas.

use smithay::backend::input::*;

use crate::state::BookosComp;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dispositivo;

impl Device for Dispositivo {
    fn id(&self) -> String {
        "gestos-de-prueba".into()
    }
    fn name(&self) -> String {
        "Touchpad del guion".into()
    }
    fn has_capability(&self, capacidad: DeviceCapability) -> bool {
        capacidad == DeviceCapability::Gesture
    }
    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }
    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

pub struct Backend;

impl InputBackend for Backend {
    type Device = Dispositivo;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;
    type GestureSwipeBeginEvent = Evento;
    type GestureSwipeUpdateEvent = Evento;
    type GestureSwipeEndEvent = Evento;
    type GesturePinchBeginEvent = Evento;
    type GesturePinchUpdateEvent = Evento;
    type GesturePinchEndEvent = Evento;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = ();
}

#[derive(Debug)]
pub struct Evento {
    tiempo: u64,
    dedos: u32,
    dx: f64,
    dy: f64,
    escala: f64,
    cancelado: bool,
}

impl Event<Backend> for Evento {
    fn time(&self) -> u64 {
        self.tiempo
    }
    fn device(&self) -> Dispositivo {
        Dispositivo
    }
}
impl GestureBeginEvent<Backend> for Evento {
    fn fingers(&self) -> u32 {
        self.dedos
    }
}
impl GestureEndEvent<Backend> for Evento {
    fn cancelled(&self) -> bool {
        self.cancelado
    }
}
impl GestureSwipeBeginEvent<Backend> for Evento {}
impl GestureSwipeEndEvent<Backend> for Evento {}
impl GestureSwipeUpdateEvent<Backend> for Evento {
    fn delta_x(&self) -> f64 {
        self.dx
    }
    fn delta_y(&self) -> f64 {
        self.dy
    }
}
impl GesturePinchBeginEvent<Backend> for Evento {}
impl GesturePinchEndEvent<Backend> for Evento {}
impl GesturePinchUpdateEvent<Backend> for Evento {
    fn delta_x(&self) -> f64 {
        self.dx
    }
    fn delta_y(&self) -> f64 {
        self.dy
    }
    fn scale(&self) -> f64 {
        self.escala
    }
    fn rotation(&self) -> f64 {
        0.0
    }
}

// añadido en la 0.2.0 -- las órdenes separadas dejan capturar con el dedo todavía apoyado.
pub fn orden(state: &mut BookosComp, partes: &[&str]) -> Result<usize, &'static str> {
    let mut event = Evento {
        tiempo: state.start_time.elapsed().as_micros() as u64,
        dedos: 4,
        dx: 0.0,
        dy: 0.0,
        escala: 1.0,
        cancelado: false,
    };
    let (evento, fijas) = match partes {
        ["deslizar", "inicio", dedos, ..] => {
            event.dedos = dedos.parse().map_err(|_| "número de dedos inválido")?;
            (InputEvent::GestureSwipeBegin { event }, 3)
        }
        ["deslizar", "avance", dx, dy, ..] => {
            event.dx = dx
                .parse()
                .map_err(|_| "desplazamiento horizontal inválido")?;
            event.dy = dy.parse().map_err(|_| "desplazamiento vertical inválido")?;
            (InputEvent::GestureSwipeUpdate { event }, 4)
        }
        ["deslizar", fin @ ("fin" | "cancelar"), ..] => {
            event.cancelado = *fin == "cancelar";
            (InputEvent::GestureSwipeEnd { event }, 2)
        }
        ["pellizcar", "inicio", dedos, ..] => {
            event.dedos = dedos.parse().map_err(|_| "número de dedos inválido")?;
            (InputEvent::GesturePinchBegin { event }, 3)
        }
        ["pellizcar", "avance", escala, ..] => {
            event.escala = escala.parse().map_err(|_| "escala inválida")?;
            (InputEvent::GesturePinchUpdate { event }, 3)
        }
        ["pellizcar", fin @ ("fin" | "cancelar"), ..] => {
            event.cancelado = *fin == "cancelar";
            (InputEvent::GesturePinchEnd { event }, 2)
        }
        _ => return Err("gesto de guion inválido"),
    };
    crate::input::handle::<Backend>(state, evento);
    Ok(fijas)
}
