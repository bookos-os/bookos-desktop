//! Qué celdas enseñan los conmutadores y en qué orden.
//!
//! Está separado de [`crate::keybinds`] y **no conoce `Window`** a propósito: un
//! `Window` de Smithay no se puede construir sin un display, así que la lógica
//! metida allí no se podría probar. Aquí entran datos planos —el sello de foco
//! de cada ventana— y sale el orden; el que traduce índices a ventanas es el
//! compositor.
//!
//! El compositor reúne los índices de una misma unidad de trabajo después de
//! ordenar: cada grupo y sus diálogos ocupan una celda, y dos documentos
//! independientes de una aplicación siguen teniendo una celda cada uno.

use smithay::utils::{Logical, Point, Rectangle};
use std::time::Instant;

/// Ordena las ventanas por uso reciente y devuelve sus índices.
///
/// `sellos[i]` es cuándo se enfocó por última vez la ventana `i`, o `None` si
/// nunca ha tenido el foco. Las que nunca lo han tenido van **al final**, en el
/// orden en que llegaron: son ventanas recién abiertas que el usuario todavía no
/// ha usado, y ponerlas delante convertiría el primer Alt+Tab en una sorpresa.
///
/// Se trunca a `MAXIMO` celdas porque a partir de ahí la fila no cabe en la
/// pantalla; el corte se hace **después** de ordenar, así que lo que se pierde
/// es lo más viejo, no lo primero que se encontró.
// 0.1.0
pub fn orden(sellos: &[Option<Instant>], maximo: usize) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..sellos.len()).collect();
    // La ordenación es **estable**, así que las que empatan —las que nunca han
    // tenido el foco— conservan su orden de llegada sin tener que decirlo.
    indices.sort_by(|a, b| match (sellos[*a], sellos[*b]) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    indices.truncate(maximo);
    indices
}

/// Dónde se pinta de verdad una ventana de la exposición mientras viaja desde
/// su `sitio` en el escritorio hasta su `hueco` en la rejilla.
// añadido en la 0.2.0
pub fn hueco_pintado(
    sitio: Option<Rectangle<i32, Logical>>,
    hueco: Rectangle<i32, Logical>,
    origen_salida: Point<i32, Logical>,
    entrada: f64,
) -> Rectangle<i32, Logical> {
    let Some(sitio) = sitio.filter(|_| entrada < 1.0) else {
        return hueco;
    };
    let mezclar = |a: i32, b: i32| a + ((b - a) as f64 * entrada).round() as i32;
    let desde = sitio.loc - origen_salida;
    Rectangle::new(
        (mezclar(desde.x, hueco.loc.x), mezclar(desde.y, hueco.loc.y)).into(),
        (
            mezclar(sitio.size.w, hueco.size.w),
            mezclar(sitio.size.h, hueco.size.h),
        )
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Sellos a partir de segundos: cuanto mayor el número, más reciente.
    fn sellos(v: &[Option<u64>]) -> Vec<Option<Instant>> {
        let base = Instant::now();
        v.iter()
            .map(|s| s.map(|s| base + Duration::from_secs(s)))
            .collect()
    }

    /// La más reciente va primera: es la que tienes delante, y el conmutador
    /// arranca señalando la **segunda**, que es a la que quieres ir.
    #[test]
    fn la_mas_reciente_va_primera() {
        let s = sellos(&[Some(10), Some(30), Some(20)]);
        assert_eq!(orden(&s, 12), vec![1, 2, 0]);
    }

    /// Dos ventanas de la **misma** aplicación son dos celdas también en Alt.
    /// Es el caso que dejaba el atajo sin hacer nada cuando agrupaba.
    #[test]
    fn dos_ventanas_de_la_misma_app_son_dos_celdas() {
        let s = sellos(&[Some(10), Some(30), Some(20)]);
        assert_eq!(orden(&s, 12).len(), 3, "ninguna se funde con otra");
    }

    /// Una ventana recién abierta y todavía sin foco va al final: ponerla
    /// delante haría que el primer Alt+Tab llevara a donde no se espera.
    #[test]
    fn las_que_nunca_tuvieron_foco_van_al_final() {
        let s = sellos(&[None, Some(10), None, Some(20)]);
        assert_eq!(
            orden(&s, 12),
            vec![3, 1, 0, 2],
            "y entre ellas, por llegada"
        );
    }

    /// El corte deja lo más reciente, no lo primero que se encontró.
    #[test]
    fn el_truncado_se_queda_con_lo_reciente() {
        let s = sellos(&[Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(orden(&s, 2), vec![3, 2]);
    }

    /// Sin ventanas no hay celdas: el llamador no abrirá nada.
    #[test]
    fn sin_ventanas_no_hay_celdas() {
        assert!(orden(&[], 12).is_empty());
        assert_eq!(orden(&sellos(&[Some(1)]), 12), vec![0], "una sola tampoco");
    }

    #[test]
    fn el_hueco_pintado_va_del_sitio_a_la_rejilla() {
        let sitio = Rectangle::new((100, 100).into(), (1000, 600).into());
        let hueco = Rectangle::new((0, 0).into(), (500, 300).into());
        let origen = Point::from((0, 0));
        assert_eq!(hueco_pintado(Some(sitio), hueco, origen, 0.0), sitio);
        assert_eq!(hueco_pintado(Some(sitio), hueco, origen, 1.0), hueco);
        assert_eq!(
            hueco_pintado(Some(sitio), hueco, origen, 0.5),
            Rectangle::new((50, 50).into(), (750, 450).into())
        );
    }
}
