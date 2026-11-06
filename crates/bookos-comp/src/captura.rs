//! `zwlr_screencopy_v1`: copiar una salida a un buffer del cliente.
//!
//! Es lo que hace posible **hacer una captura de pantalla y compartir
//! pantalla**, que hasta ahora no se podía ninguna de las dos cosas. No hace
//! falta escribir un backend de portal: `xdg-desktop-portal-wlr` habla este
//! mismo protocolo, así que con esto puesto funcionan tanto `grim` como el
//! selector de «compartir pantalla» de un navegador.
//!
//! ## Por qué a mano y no con Smithay
//!
//! Smithay 0.7 no trae este protocolo. Lo que sí trae es el XML generado
//! —`wayland_protocols_wlr::screencopy`, que ya entra en el árbol con
//! `data_control`— y `RenderFrameResult::blit_frame_result`, que es justo el
//! camino pensado para esto. O sea que no hay dependencia nueva: lo que falta
//! son los `Dispatch`, que es este fichero.
//!
//! ## Cuándo se copia
//!
//! No al recibir `copy`, sino **después del siguiente fotograma**. La copia
//! sale del resultado de dibujar la salida, que es lo que ya está en la GPU;
//! atenderla en el momento obligaría a componer la escena otra vez solo para
//! esto. El cliente pide, se apunta en `pendientes`, y el backend la sirve al
//! terminar de dibujar esa salida.
//!
//! ## Qué no hace todavía
//!
//! - **Solo `wl_shm`.** El evento `linux_dmabuf` no se anuncia, así que el
//!   cliente pide memoria compartida. Para una captura da igual; para compartir
//!   pantalla a 60 fps sería una copia GPU→CPU→GPU que se puede quitar después.
//! - **`copy_with_damage` se sirve como `copy`**: se contesta con el daño de la
//!   salida entera. Es correcto —el daño anunciado puede ser de más, nunca de
//!   menos— y evita llevar la cuenta por cliente.

use std::sync::Mutex;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};

use crate::state::BookosComp;

/// La versión que se anuncia. La 3 es la que pide `xdg-desktop-portal-wlr`.
const VERSION: u32 = 3;

/// El formato en el que se entrega la copia.
///
/// `Xrgb8888` y no `Argb8888`: lo que sale de componer una salida es opaco —el
/// fondo tapa siempre— y anunciar alfa haría que el visor de la captura
/// multiplicara por un canal que no significa nada. Es también lo que esperan
/// `grim` y el portal.
const FORMATO: wl_shm::Format = wl_shm::Format::Xrgb8888;

/// El global vivo. Soltarlo lo retira y los clientes dejarían de ver el
/// protocolo, así que hay que guardarlo aunque nadie lo lea.
#[derive(Debug)]
pub struct CapturaState {
    #[allow(dead_code)]
    global: smithay::reexports::wayland_server::backend::GlobalId,
}

impl CapturaState {
    // 0.1.0
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<BookosComp, ZwlrScreencopyManagerV1, _>(VERSION, ());
        Self { global }
    }
}

/// Lo que el compositor guarda de cada `zwlr_screencopy_frame_v1`.
#[derive(Debug)]
pub struct DatosFrame {
    output: Output,
    /// La región pedida, en píxeles **del buffer** de la salida. La captura
    /// entera es la salida completa.
    region: Rectangle<i32, BufferCoord>,
    /// Si hay que dibujar el cursor encima.
    con_cursor: bool,
    /// Se sirve como mucho una vez: el segundo `copy` es un error de protocolo.
    servido: Mutex<bool>,
}

/// Una copia pedida y todavía sin servir.
#[derive(Debug)]
pub struct Pendiente {
    pub frame: ZwlrScreencopyFrameV1,
    pub output: Output,
    pub region: Rectangle<i32, BufferCoord>,
    /// Si el cliente pidió el cursor encima.
    ///
    /// Hoy da igual: el cursor se compone dentro de la escena como un elemento
    /// más y separarlo pediría componerla dos veces, así que sale siempre. Se
    /// guarda porque es lo que pidió el cliente, y quitar el campo escondería
    /// que no se le está haciendo caso.
    #[allow(dead_code)]
    pub con_cursor: bool,
    pub buffer: smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for BookosComp {
    // 0.1.0
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        recurso: New<ZwlrScreencopyManagerV1>,
        _datos: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(recurso, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for BookosComp {
    // 0.1.0
    fn request(
        state: &mut Self,
        _client: &Client,
        _recurso: &ZwlrScreencopyManagerV1,
        peticion: zwlr_screencopy_manager_v1::Request,
        _datos: &(),
        _dh: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_manager_v1::Request;
        let (frame, overlay_cursor, output, region) = match peticion {
            Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor, output, None),
            Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                overlay_cursor,
                output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
            ),
            Request::Destroy => return,
            _ => return,
        };

        let Some(output) = Output::from_resource(&output) else {
            // La salida se desenchufó entre que el cliente la pidió y esto:
            // se contesta `failed` en vez de dejarlo esperando para siempre.
            let frame = init.init(frame, DatosFrame::muerto());
            frame.failed();
            return;
        };
        let tamano = tamano_de(&output);
        let completa = Rectangle::from_size(tamano);
        // Recortada contra la salida: una región que se sale devolvería filas
        // de fuera del framebuffer, y eso es leer memoria que no es nuestra.
        let region = region
            .map(|r| Rectangle::new(r.loc, r.size).intersection(completa))
            .unwrap_or(Some(completa));
        let Some(region) = region.filter(|r| r.size.w > 0 && r.size.h > 0) else {
            let frame = init.init(frame, DatosFrame::muerto());
            frame.failed();
            return;
        };

        let frame = init.init(
            frame,
            DatosFrame {
                output,
                region,
                con_cursor: overlay_cursor != 0,
                servido: Mutex::new(false),
            },
        );
        // Cuatro bytes por píxel: es lo que mide `Xrgb8888`.
        let stride = region.size.w as u32 * 4;
        frame.buffer(FORMATO, region.size.w as u32, region.size.h as u32, stride);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
        let _ = state;
    }
}

impl DatosFrame {
    /// Un frame que ya nace fallado: hace falta porque el protocolo obliga a
    /// crear el objeto antes de poder decirle que no.
    // 0.1.0
    fn muerto() -> Self {
        Self {
            output: Output::new(
                "muerta".into(),
                smithay::output::PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: "BookOS".into(),
                    model: "ninguna".into(),
                },
            ),
            region: Rectangle::default(),
            con_cursor: false,
            servido: Mutex::new(true),
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, DatosFrame> for BookosComp {
    // 0.1.0
    fn request(
        state: &mut Self,
        _client: &Client,
        recurso: &ZwlrScreencopyFrameV1,
        peticion: zwlr_screencopy_frame_v1::Request,
        datos: &DatosFrame,
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_frame_v1::Request;
        let buffer = match peticion {
            Request::Copy { buffer } | Request::CopyWithDamage { buffer } => buffer,
            Request::Destroy => {
                // Se olvida la petición pendiente: el cliente se ha ido y su
                // buffer ya no es válido.
                state.capturas_pantalla.retain(|p| p.frame != *recurso);
                return;
            }
            _ => return,
        };

        {
            let mut servido = datos.servido.lock().expect("cerrojo del frame de captura");
            if *servido {
                recurso.post_error(
                    zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                    "este frame ya se había copiado",
                );
                return;
            }
            *servido = true;
        }

        // El buffer tiene que ser de memoria compartida y del tamaño y formato
        // que se anunciaron: es lo único que se sabe rellenar.
        let esperado = (datos.region.size.w, datos.region.size.h);
        match smithay::wayland::shm::with_buffer_contents(&buffer, |_, _, datos_shm| {
            (datos_shm.width, datos_shm.height, datos_shm.format)
        }) {
            Ok((w, h, formato)) if (w, h) == esperado && formato == FORMATO => {}
            Ok(_) => {
                recurso.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "el buffer no tiene el tamaño o el formato que se anunciaron",
                );
                return;
            }
            Err(_) => {
                recurso.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "se esperaba un buffer de wl_shm",
                );
                return;
            }
        }

        // Se apunta y se sirve tras el próximo fotograma de esa salida. Y se
        // pide dibujar: si el escritorio está quieto no habría fotograma que
        // copiar y la captura se quedaría esperando.
        state.capturas_pantalla.push(Pendiente {
            frame: recurso.clone(),
            output: datos.output.clone(),
            region: datos.region,
            con_cursor: datos.con_cursor,
            buffer,
        });
        state.needs_redraw = true;
    }

    // 0.1.0
    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        recurso: &ZwlrScreencopyFrameV1,
        _datos: &DatosFrame,
    ) {
        state.capturas_pantalla.retain(|p| p.frame != *recurso);
    }
}

/// ¿Es el compositor el dueño del portapapeles y ofrece una imagen?
///
/// Para el autotest. **No pega**: Smithay no deja leer la propia selección del
/// servidor —`request_data_device_client_selection` devuelve
/// `ServerSideSelection` a propósito, porque esa función existe para que el
/// compositor lea la selección de un *cliente*—. Y ese error es justo la
/// respuesta que se busca: dice que el dueño somos nosotros y que el tipo que
/// se pregunta está entre los ofrecidos. Sin selección puesta contestaría
/// `NoSelection`, y con otra lista de tipos, `InvalidMimetype`.
///
/// Que los bytes lleguen de verdad se comprueba con un cliente que pegue; eso
/// no cabe aquí dentro.
// 0.1.0
pub fn el_portapapeles_tiene_imagen(state: &crate::state::BookosComp) -> bool {
    use smithay::reexports::rustix;
    use smithay::wayland::selection::data_device::{
        SelectionRequestError, request_data_device_client_selection,
    };
    let Ok((_lectura, escritura)) = rustix::pipe::pipe() else {
        return false;
    };
    match request_data_device_client_selection(&state.seat, "image/png".into(), escritura) {
        Err(SelectionRequestError::ServerSideSelection) => true,
        otro => {
            tracing::warn!(?otro, "el portapapeles no ofrece image/png del compositor");
            false
        }
    }
}

/// Pasa un rectángulo con el origen **arriba** a la izquierda al que espera
/// `copy_framebuffer`, que según la salida lo tiene **abajo**.
///
/// `GlesRenderer::copy_framebuffer` le da la `y` de la región tal cual a
/// `glReadPixels`, que cuenta filas desde la primera de la memoria. Dónde cae
/// esa fila depende del `Transform` de la salida, igual que la vuelta de filas
/// de [`hay_que_voltear`], y las dos van juntas:
///
/// - Con la escena volteada (winit, `Flipped180`) la fila 0 es el borde de
///   **abajo**: hay que reflejar la `y`. Medido en anidado, un recuadro pedido
///   en y=262 salía con el contenido de y=1014, que es `1801 − 262 − 525`.
/// - En `Transform::Normal` (udev) la fila 0 ya es la de arriba: reflejarla
///   —que es lo que se hacía siempre— devolvía la franja simétrica, y es por
///   lo que en la sesión real la captura salía de otro sitio que el marcado.
///   La pantalla entera no lo notaba porque su rectángulo es simétrico.
// 0.1.0
pub fn region_gl(
    region: Rectangle<i32, BufferCoord>,
    alto_total: i32,
    volteada: bool,
) -> Rectangle<i32, BufferCoord> {
    if !volteada {
        return region;
    }
    Rectangle::new(
        (region.loc.x, alto_total - region.loc.y - region.size.h).into(),
        region.size,
    )
}

/// ¿Hay que darle la vuelta a las filas de esta salida al leerlas del
/// framebuffer?
///
/// **No se puede preguntar a `TextureMapping::flipped`**, que es lo que se hacía
/// y es la causa de que las capturas salieran espejadas en vertical: en el
/// renderizador GLES de Smithay esa función devuelve la constante `true`, así
/// que la vuelta se daba siempre.
///
/// Lo que decide de verdad es el `Transform` de la salida, porque es lo que
/// orienta la escena dentro del framebuffer. Las dos que hay:
///
/// - **udev**, la sesión de verdad, va en `Transform::Normal`: la escena se
///   dibuja de arriba abajo y las filas salen ya en orden. Voltearlas era
///   justo lo que rompía la captura.
/// - **winit**, el compositor anidado para desarrollo, va en
///   `Transform::Flipped180` —lo pide la superficie EGL del anfitrión—, y ahí
///   la escena sí queda al revés y hay que darle la vuelta.
///
/// Por eso el fallo no se veía probando en anidado, que es donde se comprobó
/// en su día: las dos vueltas se compensaban y el PNG salía bien.
// 0.1.0
pub fn hay_que_voltear(output: &Output) -> bool {
    output.current_transform().flipped()
}

/// El tamaño del buffer de una salida, que es lo que se copia.
// 0.1.0
pub fn tamano_de(output: &Output) -> Size<i32, BufferCoord> {
    let modo = output.current_mode().map(|m| m.size).unwrap_or_default();
    let transformado = output.current_transform().transform_size(modo);
    (transformado.w, transformado.h).into()
}

/// Copia los píxeles al buffer del cliente y le dice que ya está.
///
/// `pixeles` viene tal cual lo devuelve `map_texture`: empaquetado, cuatro
/// bytes por píxel y tantas filas como alto tiene la región. El buffer del
/// cliente puede tener un `stride` mayor que su ancho —lo elige él—, así que se
/// copia **fila a fila** y no de un tirón.
///
/// Con `invertida`, las filas de `pixeles` vienen de abajo arriba y se le dan
/// la vuelta al copiarlas. Lo decide [`hay_que_voltear`], **no**
/// `TextureMapping::flipped`, que es una constante y por eso volteaba las
/// capturas de la sesión de verdad. Se invierte aquí y no con la bandera
/// `YInvert` del protocolo porque la copia ya va fila a fila: darles la vuelta
/// no cuesta nada y así el cliente recibe siempre lo mismo, sin depender de que
/// sepa interpretar la bandera.
///
/// Devuelve `false` si el buffer se murió entre medias, que pasa cuando el
/// cliente se va justo después de pedir la copia.
// 0.1.0
pub fn volcar(pendiente: &Pendiente, pixeles: &[u8], invertida: bool) -> bool {
    let (ancho, alto) = (
        pendiente.region.size.w as usize,
        pendiente.region.size.h as usize,
    );
    let fila = ancho * 4;
    if pixeles.len() < fila * alto {
        tracing::error!(
            tiene = pixeles.len(),
            hacen_falta = fila * alto,
            "la copia de la pantalla salió más corta de lo que mide la región"
        );
        pendiente.frame.failed();
        return false;
    }
    let escrito = smithay::wayland::shm::with_buffer_contents_mut(
        &pendiente.buffer,
        |destino, largo, datos| {
            let stride = datos.stride as usize;
            // SAFETY: `with_buffer_contents_mut` da un puntero al mapeo del
            // cliente y su largo; se escribe solo dentro de esos límites, que
            // es lo que comprueban el `min` y el `largo` de abajo.
            let destino = unsafe { std::slice::from_raw_parts_mut(destino, largo) };
            for y in 0..alto {
                let origen = if invertida { alto - 1 - y } else { y };
                let (o, d) = (origen * fila, y * stride);
                if d + fila > destino.len() {
                    break;
                }
                destino[d..d + fila].copy_from_slice(&pixeles[o..o + fila]);
            }
        },
    );
    if escrito.is_err() {
        // El buffer se fue con su cliente: no hay a quién contestarle.
        return false;
    }
    true
}

/// Le dice al cliente que su copia está lista.
///
/// El instante va en el reloj monotónico, que es el mismo con el que se anuncia
/// `wp_presentation`: para un cliente que graba, las dos marcas tienen que
/// poder compararse.
// 0.1.0
pub fn contestar(pendiente: &Pendiente, tamano: Size<i32, BufferCoord>) {
    use zwlr_screencopy_frame_v1::Flags;
    // Sin `YInvert`: la vuelta se la damos nosotros en `volcar`, así que lo que
    // el cliente tiene en el buffer ya está de arriba abajo.
    pendiente.frame.flags(Flags::empty());
    if pendiente.frame.version() >= 2 {
        pendiente
            .frame
            .damage(0, 0, tamano.w.max(0) as u32, tamano.h.max(0) as u32);
    }
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = ahora.as_secs();
    pendiente.frame.ready(
        (secs >> 32) as u32,
        (secs & 0xFFFF_FFFF) as u32,
        ahora.subsec_nanos(),
    );
}

/// El velo de la capa de captura, con el agujero y el marco **redondeados**.
///
/// Un shader y no rectángulos de color porque una esquina curva no se hace con
/// rectángulos. Es un solo elemento a pantalla completa que calcula, píxel a
/// píxel, la distancia al rectángulo redondeado: fuera oscurece, en la franja
/// del borde pinta el acento y dentro no pinta nada. El borde sale suavizado
/// sin coste, que con rectángulos tampoco se podía.
///
/// El elemento vive aquí y no se crea en cada fotograma: su `Id` y su contador
/// de commits son los que le dicen al damage tracker si ha cambiado, y solo
/// se tocan cuando cambian el recuadro, la pantalla o el acento.
pub struct Velo {
    elemento: smithay::backend::renderer::gles::element::PixelShaderElement,
    /// Lo último que se le pasó al shader, para no subir el contador de commits
    /// —y repintar la pantalla entera— cuando no ha cambiado nada.
    ultimo: Option<[f32; 11]>,
}

const FRAGMENTO_VELO: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// En lógicos: x, y, ancho, alto. Ancho 0 es «no hay nada marcado».
uniform vec4 recuadro;
uniform float radio;
uniform float borde;
// Cuántos píxeles físicos mide un lógico: el suavizado es de un píxel físico.
uniform float escala;
uniform float velo;
uniform vec3 acento;

// Distancia con signo a un rectángulo redondeado: negativa dentro.
float caja(vec2 p, vec2 centro, vec2 medio, float r) {
    vec2 q = abs(p - centro) - medio + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 p = v_coords * size;
    vec4 color = vec4(0.0, 0.0, 0.0, velo);
    if (recuadro.z > 0.0) {
        float aa = 0.5 / escala;
        vec2 medio = recuadro.zw * 0.5;
        float r = min(radio, min(medio.x, medio.y));
        float d = caja(p, recuadro.xy + medio, medio, r);
        float dentro = 1.0 - smoothstep(-aa, aa, d);
        // El marco va **dentro** del recuadro, como iba el borde de iced: lo
        // que queda bajo la línea sí sale en la foto.
        float marco = dentro * smoothstep(-borde - aa, -borde + aa, d);
        color = vec4(0.0, 0.0, 0.0, velo) * (1.0 - dentro) + vec4(acento, 1.0) * marco;

        // Las asas: un trazo claro por fuera de cada esquina que sigue su
        // curva, para que se vea que de ahí se tira. Por fuera y no encima del
        // marco porque lo de dentro sale en la foto. Van en lógicos: 2 de aire,
        // 4 de grueso y 16 de largo desde la esquina, que quedan dentro de los
        // 8 de agarre de `bookos_shell::captura` y se leen a cualquier escala.
        vec2 e = abs(p - recuadro.xy - medio) - medio;
        float esquina = step(-16.0, e.x) * step(-16.0, e.y);
        float asa = esquina
            * smoothstep(2.0 - aa, 2.0 + aa, d)
            * (1.0 - smoothstep(6.0 - aa, 6.0 + aa, d));
        color = mix(color, vec4(0.87, 0.87, 0.87, 1.0), asa);
    }
    gl_FragColor = color * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.3, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

impl Velo {
    /// Compila el shader. `None` si el driver no lo acepta: el velo sale igual,
    /// con las esquinas rectas de `ShellHost::velo_captura`.
    // 0.1.0
    pub fn new(renderer: &mut smithay::backend::renderer::gles::GlesRenderer) -> Option<Self> {
        use smithay::backend::renderer::element::Kind;
        use smithay::backend::renderer::gles::element::PixelShaderElement;
        use smithay::backend::renderer::gles::{UniformName, UniformType};

        let programa = renderer
            .compile_custom_pixel_shader(
                FRAGMENTO_VELO,
                &[
                    UniformName::new("recuadro", UniformType::_4f),
                    UniformName::new("radio", UniformType::_1f),
                    UniformName::new("borde", UniformType::_1f),
                    UniformName::new("escala", UniformType::_1f),
                    UniformName::new("velo", UniformType::_1f),
                    UniformName::new("acento", UniformType::_3f),
                ],
            )
            .inspect_err(|err| {
                tracing::warn!(
                    "el velo de la captura irá sin redondear, el driver no compiló el shader: {err}"
                )
            })
            .ok()?;
        Some(Self {
            elemento: PixelShaderElement::new(
                programa,
                Rectangle::default(),
                None,
                1.0,
                Vec::new(),
                Kind::Unspecified,
            ),
            ultimo: None,
        })
    }

    /// El elemento para este fotograma. `pantalla` va en físicos y el recuadro
    /// en lógicos, que es lo que da `ShellHost::datos_velo_captura`.
    // 0.1.0
    pub fn elemento(
        &mut self,
        pantalla: (i32, i32),
        escala: f64,
        marcado: Option<bookos_shell::captura::Recuadro>,
    ) -> smithay::backend::renderer::gles::element::PixelShaderElement {
        use bookos_shell::captura::{BORDE, VELO};
        use smithay::backend::renderer::gles::Uniform;

        // Hacia arriba: con escala fraccional, redondear a lo más cercano podía
        // dejar la última fila de píxeles de la pantalla sin velo.
        let logico = (
            (pantalla.0 as f64 / escala).ceil() as i32,
            (pantalla.1 as f64 / escala).ceil() as i32,
        );
        let r = marcado.map_or([0.0; 4], |r| [r.x, r.y, r.w, r.h]);
        let acento = bookos_shell::tema::acento();
        let valores = [
            r[0],
            r[1],
            r[2],
            r[3],
            logico.0 as f32,
            logico.1 as f32,
            escala as f32,
            acento.r,
            acento.g,
            acento.b,
            // El radio de un control del sistema: el recuadro es una selección
            // que se toca, no una tarjeta.
            bookos_shell::tema::R_CONTROL,
        ];
        if self.ultimo != Some(valores) {
            self.elemento
                .resize(Rectangle::from_size(logico.into()), None);
            self.elemento.update_uniforms(vec![
                Uniform::new("recuadro", (r[0], r[1], r[2], r[3])),
                Uniform::new("radio", valores[10]),
                Uniform::new("borde", BORDE),
                Uniform::new("escala", escala as f32),
                Uniform::new("velo", VELO),
                Uniform::new("acento", (acento.r, acento.g, acento.b)),
            ]);
            self.ultimo = Some(valores);
        }
        self.elemento.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Transform;

    fn salida(transform: Transform) -> Output {
        let output = Output::new(
            "prueba".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "BookOS".into(),
                model: "prueba".into(),
            },
        );
        output.change_current_state(None, Some(transform), None, None);
        output
    }

    #[test]
    fn el_recorte_solo_se_refleja_con_la_escena_volteada() {
        let r = Rectangle::<i32, BufferCoord>::new((10, 262).into(), (100, 525).into());
        assert_eq!(region_gl(r, 1801, false), r);
        assert_eq!(region_gl(r, 1801, true).loc.y, 1801 - 262 - 525);
    }

    /// Lo que decide la vuelta es la orientación de la salida, y las dos que hay
    /// en marcha dan respuestas distintas. Este test existe porque durante un
    /// tiempo la decisión salió de `TextureMapping::flipped`, que es la
    /// constante `true`, y las capturas de la sesión de verdad salían espejadas
    /// mientras que las del anidado —donde se probaba— salían bien.
    #[test]
    fn la_vuelta_depende_de_la_orientacion_de_la_salida() {
        // udev, la sesión de verdad.
        assert!(!hay_que_voltear(&salida(Transform::Normal)));
        // winit, el compositor anidado de desarrollo.
        assert!(hay_que_voltear(&salida(Transform::Flipped180)));
        // Una pantalla girada no está espejada: no lleva vuelta.
        assert!(!hay_que_voltear(&salida(Transform::_90)));
        assert!(!hay_que_voltear(&salida(Transform::_180)));
    }
}
