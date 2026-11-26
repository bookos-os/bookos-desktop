//! Geometría compartida por las pantallas de carga del greeter y del compositor.
//! Al ceder el VT, la marca y la barra deben seguir en el mismo sitio.

pub const LOGO: i32 = 104;
pub const ANCHO_BARRA: i32 = 200;
pub const ALTO_BARRA: i32 = 5;
pub const SEPARACION: i32 = 28;
pub const DESFASE_Y: i32 = 6;

pub const PROGRESO_INICIAL: f32 = 0.20;
pub const PROGRESO_REDUCIDO: f32 = 0.50;
pub const PISTA: [u8; 4] = [255, 255, 255, 42];
pub const RELLENO: [u8; 4] = [244, 247, 255, 236];
