//! Un recorte de verdad para contenido que se desplaza dentro de una ventana
//! fija.
//!
//! `container(..).clip(true)` y `column(..).clip(true)` de iced **no recortan
//! lo que se pinta**: solo estrechan el `viewport` que reciben los hijos, y
//! los hijos que son imágenes o texto lo ignoran. Lo único que descarta es la
//! fila que queda entera fuera. Con una rejilla que se desplaza, eso se ve
//! como filas que se montan sobre la cabecera y desaparecen de golpe en lugar
//! de deslizarse bajo el borde.
//!
//! Aquí el recorte es una capa del renderizador (`with_layer`), que es lo que
//! sí corta. Además el contenido se mide **sin el alto de la ventana**: con el
//! tope puesto, las filas que asoman por debajo se encogían para caber en lo
//! que quedaba, y salían diminutas en vez de cortadas.

use iced_core::widget::{Tree, tree};
use iced_core::{Element, Layout, Length, Point, Rectangle, Size, Widget, layout, mouse, renderer};

pub struct Recorte<'a, Message, Theme, Renderer> {
    contenido: Element<'a, Message, Theme, Renderer>,
    ancho: f32,
    alto: f32,
    /// Cuánto se ha subido el contenido: lo que ya no se ve por arriba.
    desplazamiento: f32,
}

impl<'a, Message, Theme, Renderer> Recorte<'a, Message, Theme, Renderer> {
    // añadido en la 0.2.0
    pub fn new(
        contenido: impl Into<Element<'a, Message, Theme, Renderer>>,
        ancho: f32,
        alto: f32,
        desplazamiento: f32,
    ) -> Self {
        Self {
            contenido: contenido.into(),
            ancho,
            alto,
            desplazamiento,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Recorte<'_, Message, Theme, Renderer>
where
    Renderer: iced_core::Renderer,
{
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.ancho), Length::Fixed(self.alto))
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.contenido)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.contenido));
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::stateless()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        let sin_tope = layout::Limits::new(Size::ZERO, Size::new(self.ancho, f32::INFINITY));
        let hijo = self
            .contenido
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &sin_tope)
            .move_to(Point::new(0.0, -self.desplazamiento));
        layout::Node::with_children(Size::new(self.ancho, self.alto), vec![hijo])
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(visible) = layout.bounds().intersection(viewport) else {
            return;
        };
        renderer.with_layer(visible, |renderer| {
            self.contenido.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout
                    .children()
                    .next()
                    .expect("el recorte lleva su contenido"),
                cursor,
                &visible,
            );
        });
    }
}

impl<'a, Message, Theme, Renderer> From<Recorte<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced_core::Renderer + 'a,
{
    fn from(recorte: Recorte<'a, Message, Theme, Renderer>) -> Self {
        Element::new(recorte)
    }
}
