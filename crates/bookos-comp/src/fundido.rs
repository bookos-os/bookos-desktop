//! El paso de claro a oscuro, y al revés, fundido en vez de a golpe.
//!
//! El fondo de pantalla ya se fundía solo (`fondo_saliente`), pero el panel, el
//! dock y las tarjetas cambiaban de negro a blanco en un fotograma. Aquí se
//! fotografía la pantalla **con el tema viejo** justo antes de pintar el nuevo
//! y esa foto se pinta encima, desvaneciéndose.
//!
//! ## Cómo se consigue la foto del tema viejo
//!
//! Cuando llega el cambio el renderizador no está a mano —lo piden la tarjeta
//! de Apariencia, el reloj o Ajustes por D-Bus, y cada uno hace después cosas
//! que dependen de que el tema ya esté puesto—, así que el cambio se aplica al
//! momento y solo se apunta cuál había. En el fotograma siguiente la escena
//! vuelve a poner un instante el tema viejo, se compone en una textura y se
//! repone el nuevo. Son dos repintados del shell de más, una sola vez.
//!
//! Mientras dura ese instante el hilo del portal podría leer el tema viejo: el
//! tema es un estático del proceso. La ventana es de unos milisegundos y lo
//! peor que pasa es que una aplicación de fuera que pregunte justo entonces
//! reciba el tema de antes hasta la señal siguiente.

use std::time::Instant;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ContextId, Frame, Offscreen, Renderer};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use bookos_shell::tema;

pub struct Fundido {
    foto: GlesTexture,
    id: Id,
    /// El tamaño lógico de la salida, que es a lo que se estira la foto.
    tamano: Size<i32, Logical>,
    /// La foto entera, que es lo que se muestrea. Hace falta decirlo: sin
    /// recorte, `TextureRenderElement` toma el tamaño **lógico** como recorte
    /// y con escala 1,75 muestreaba solo la esquina de arriba a la izquierda
    /// —1280×800 de 2240×1400— estirada a toda la pantalla.
    recorte: Rectangle<f64, Logical>,
    inicio: Instant,
}

impl Fundido {
    /// Compone `elementos` —la escena con el tema viejo— en una textura del
    /// tamaño de la salida.
    // 0.1.0
    pub fn fotografiar<E: RenderElement<GlesRenderer>>(
        renderer: &mut GlesRenderer,
        elementos: &[E],
        output: &Output,
    ) -> Option<Self> {
        let modo = output.current_mode()?;
        let escala = output.current_scale().fractional_scale();
        let fisico: Size<i32, Physical> = (modo.size.w, modo.size.h).into();
        let mut foto = renderer
            .create_buffer(Fourcc::Abgr8888, (fisico.w, fisico.h).into())
            .inspect_err(|err| tracing::warn!("no se pudo crear la foto del tema: {err}"))
            .ok()?;
        let completo = Rectangle::from_size(fisico);
        {
            let mut fb = renderer.bind(&mut foto).ok()?;
            let mut frame = renderer.render(&mut fb, fisico, Transform::Normal).ok()?;
            frame
                .clear(smithay::backend::renderer::Color32F::BLACK, &[completo])
                .ok()?;
            // De delante hacia atrás en la lista: se dibujan al revés.
            for elemento in elementos.iter().rev() {
                let dst = elemento.geometry(Scale::from(escala));
                if let Err(err) = elemento.draw(&mut frame, elemento.src(), dst, &[completo], &[]) {
                    tracing::debug!("algo no entró en la foto del tema: {err}");
                }
            }
            // Como en el genio: la foto se dibuja después en este mismo
            // contexto GL, que ya ordena las dos cosas.
            let _ = frame
                .finish()
                .inspect_err(|err| tracing::warn!("no se pudo cerrar la foto del tema: {err}"));
        }
        Some(Self {
            foto,
            id: Id::new(),
            tamano: fisico.to_f64().to_logical(escala).to_i32_round(),
            recorte: Rectangle::from_size((fisico.w as f64, fisico.h as f64).into()),
            inicio: Instant::now(),
        })
    }

    // 0.1.0
    pub fn terminado(&self) -> bool {
        self.inicio.elapsed() >= tema::D_TEMA
    }

    /// La foto de este fotograma, encima de todo y cada vez más transparente.
    // 0.1.0
    pub fn elemento(&self, contexto: &ContextId<GlesTexture>) -> TextureRenderElement<GlesTexture> {
        let t = tema::C_TEMA.eval(tema::avance(self.inicio.elapsed(), tema::D_TEMA));
        TextureRenderElement::from_static_texture(
            self.id.clone(),
            contexto.clone(),
            Point::<f64, Physical>::from((0.0, 0.0)),
            self.foto.clone(),
            1,
            Transform::Normal,
            Some(1.0 - t),
            Some(self.recorte),
            Some(self.tamano),
            None,
            Kind::Unspecified,
        )
    }
}
