//! La animación de cerrar una ventana: la de abrir, al revés.
//!
//! ## Por qué hay que guardar las texturas
//!
//! Cuando el cliente cierra, destruye su `xdg_toplevel` y en el mismo lote de
//! mensajes su `wl_surface`: para el fotograma siguiente ya no hay nada que
//! pintar. Pero entre una cosa y otra Smithay avisa con `toplevel_destroyed`, y
//! en ese momento la superficie sigue viva con las texturas GL que se subieron
//! en su último commit. Un `GlesTexture` clonado mantiene la textura aunque la
//! superficie muera, así que basta con quedárselas en ese instante —sin
//! renderizador, que en los manejadores no está a mano— y pintarlas mientras
//! dura la animación.
//!
//! Vale para cualquier forma de cerrar: el botón de la barra, Meta+Q, el menú
//! «Salir» de la propia aplicación o una ventana que se cierra sola.
//!
//! ## Lo que no está
//!
//! - Los menús abiertos de la ventana no entran: se cierran antes que ella.
//! - Un cliente que primero desmapea —adjunta un buffer nulo— y después
//!   destruye ya no tiene texturas que guardar; se va sin animación, que es lo
//!   mismo que pasaba antes.

use std::time::{Duration, Instant};

use smithay::backend::renderer::ContextId;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::desktop::Window;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};
use smithay::wayland::seat::WaylandFocus;

use bookos_shell::tema;

/// El desvanecido. El de la entrada es `D_TARJETA`; el de salida es más corto
/// porque quien cierra ya ha decidido y no tiene nada más que mirar.
const DURACION: Duration = Duration::from_millis(180);
/// El tamaño al que se va, el mismo del que nace una ventana al abrirse.
const ZOOM_FINAL: f64 = 0.92;

/// Un trozo de la ventana: una superficie de su árbol con su textura.
struct Pieza {
    textura: GlesTexture,
    /// Identificador propio y estable: el damage tracker compara por él.
    id: Id,
    /// Dónde cae respecto a la esquina del buffer de la ventana.
    posicion: Point<i32, Logical>,
    recorte: Rectangle<f64, Logical>,
    tamano: Size<i32, Logical>,
    escala: i32,
    transformacion: Transform,
}

/// La barra de título de BookOS de una ventana decorada, que no es una
/// superficie del cliente sino un buffer del shell.
pub struct Barra {
    pub id: u64,
    pub ancho: i32,
    pub estado: bookos_shell::decoracion::Estado,
}

/// La barra de título en un fotograma del cierre: dónde iría sin zoom, el
/// centro de la ventana desde el que encoge, y el alfa y el zoom de ahora.
pub struct BarraAhora {
    pub origen: Point<f64, Physical>,
    pub centro: Point<i32, Physical>,
    pub alfa: f32,
    pub zoom: f64,
}

pub struct Cierre {
    piezas: Vec<Pieza>,
    /// Esquina del buffer y tamaño de la ventana visible, en lógicos de
    /// escritorio: el zoom se hace alrededor de su centro.
    origen: Point<f64, Logical>,
    visible: Rectangle<i32, Logical>,
    pub barra: Option<Barra>,
    inicio: Instant,
}

impl Cierre {
    /// Se queda con las texturas de la ventana tal como están ahora. `None` si
    /// no hay nada que guardar: sin contexto GL, con los efectos reducidos o
    /// con una ventana que ya no tenía buffer.
    // 0.1.0
    pub fn capturar(
        contexto: &ContextId<GlesTexture>,
        window: &Window,
        posicion: Point<f64, Logical>,
        barra: Option<Barra>,
    ) -> Option<Self> {
        if tema::efectos_reducidos() {
            return None;
        }
        let superficie = window.wl_surface()?;
        let mut piezas = Vec::new();
        with_surface_tree_downward(
            &superficie,
            Point::<i32, Logical>::from((0, 0)),
            |_, estados, posicion| {
                // Igual que `render_elements_from_surface_tree`: cada
                // subsuperficie se coloca sumando el desplazamiento de su vista
                // al de su padre, y una sin buffer corta la rama.
                let Some(datos) = estados.data_map.get::<RendererSurfaceStateUserData>() else {
                    return TraversalAction::SkipChildren;
                };
                match datos.lock().ok().and_then(|d| d.view()) {
                    Some(vista) => TraversalAction::DoChildren(*posicion + vista.offset),
                    None => TraversalAction::SkipChildren,
                }
            },
            |_, estados, posicion| {
                let Some(datos) = estados.data_map.get::<RendererSurfaceStateUserData>() else {
                    return;
                };
                let Ok(datos) = datos.lock() else {
                    return;
                };
                let (Some(vista), Some(textura)) = (datos.view(), datos.texture(contexto.clone()))
                else {
                    return;
                };
                piezas.push(Pieza {
                    textura: textura.clone(),
                    id: Id::new(),
                    posicion: *posicion + vista.offset,
                    recorte: vista.src,
                    tamano: vista.dst,
                    escala: datos.buffer_scale(),
                    transformacion: datos.buffer_transform(),
                });
            },
            |_, _, _| true,
        );
        if piezas.is_empty() {
            return None;
        }
        let geo = window.geometry();
        Some(Self {
            piezas,
            origen: posicion - geo.loc.to_f64(),
            visible: Rectangle::new(posicion.to_i32_round(), geo.size),
            barra,
            inicio: Instant::now(),
        })
    }

    // 0.1.0
    pub fn terminado(&self) -> bool {
        self.inicio.elapsed() >= DURACION
    }

    /// Alfa y zoom de ahora, con la curva de la entrada recorrida al revés.
    // 0.1.0
    fn momento(&self) -> (f32, f64) {
        let t = tema::C_SUAVE.eval(tema::avance(self.inicio.elapsed(), DURACION));
        (1.0 - t, 1.0 - (1.0 - ZOOM_FINAL) * t as f64)
    }

    /// El centro de la ventana visible, que es desde donde encoge.
    // 0.1.0
    fn centro(&self) -> Point<f64, Logical> {
        let v = self.visible.to_f64();
        (v.loc.x + v.size.w / 2.0, v.loc.y + v.size.h / 2.0).into()
    }

    /// Lleva un punto del escritorio a donde cae con el zoom de este momento.
    // 0.1.0
    fn encoger(&self, punto: Point<f64, Logical>, zoom: f64) -> Point<f64, Logical> {
        let c = self.centro();
        (c.x + (punto.x - c.x) * zoom, c.y + (punto.y - c.y) * zoom).into()
    }

    /// Los elementos de este fotograma, de delante hacia atrás.
    // 0.1.0
    pub fn elementos(
        &self,
        contexto: &ContextId<GlesTexture>,
        escala: f64,
    ) -> Vec<TextureRenderElement<GlesTexture>> {
        let (alfa, zoom) = self.momento();
        self.piezas
            .iter()
            .rev()
            .map(|p| {
                let esquina = self.encoger(self.origen + p.posicion.to_f64(), zoom);
                let tamano: Size<i32, Logical> = (
                    ((p.tamano.w as f64 * zoom).round() as i32).max(1),
                    ((p.tamano.h as f64 * zoom).round() as i32).max(1),
                )
                    .into();
                TextureRenderElement::from_static_texture(
                    p.id.clone(),
                    contexto.clone(),
                    esquina.to_physical_precise_round::<f64, f64>(escala),
                    p.textura.clone(),
                    p.escala,
                    p.transformacion,
                    Some(alfa),
                    Some(p.recorte),
                    Some(tamano),
                    None,
                    Kind::Unspecified,
                )
            })
            .collect()
    }

    /// Dónde y cómo va la barra de título ahora. `None` si la ventana no
    /// llevaba barra.
    // 0.1.0
    pub fn barra_ahora(&self, escala: f64) -> Option<BarraAhora> {
        self.barra.as_ref()?;
        let (alfa, zoom) = self.momento();
        let arriba = Point::<f64, Logical>::from((
            self.visible.loc.x as f64,
            (self.visible.loc.y - crate::decoracion::ALTO) as f64,
        ));
        Some(BarraAhora {
            origen: arriba.to_physical_precise_round(escala),
            centro: self.centro().to_physical_precise_round(escala),
            alfa,
            zoom,
        })
    }
}
