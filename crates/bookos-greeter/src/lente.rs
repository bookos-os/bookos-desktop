//! La lente de la lista de usuarios: las filas crecen al acercarse el puntero,
//! como los iconos del dock de macOS.

pub const ALTO: f32 = 48.0;
pub const HUECO: f32 = 12.0;
const AUMENTO: f32 = 0.32;
/// Hasta dónde llega la lente, en filas. Con 1,6 la vecina crece la mitad y la
/// siguiente ya no se mueve: más ancha, la lista entera respira con el ratón.
const ALCANCE: f32 = 1.6;

// 0.1.1
pub fn alto_en_reposo(filas: usize, k: f32) -> f32 {
    (filas as f32 * (ALTO + HUECO) - HUECO).max(0.0) * k
}

// 0.1.1
fn escala(distancia: f32, k: f32, fuerza: f32) -> f32 {
    let t = (distancia.abs() / (ALCANCE * (ALTO + HUECO) * k)).min(1.0);
    1.0 + AUMENTO * fuerza * (std::f32::consts::FRAC_PI_2 * t).cos().powi(2)
}

/// `(arriba, escala)` de cada fila, con el cero en el borde de arriba de la
/// lista en reposo. `foco` va en esas mismas coordenadas y `k` encoge la lista
/// entera cuando no cabe.
///
/// Las escalas salen de las posiciones **en reposo**: si salieran de las ya
/// ampliadas, mover una fila cambiaría su distancia al puntero y la lista
/// temblaría. Luego se desplaza todo para que lo que hay bajo el puntero no se
/// mueva de debajo de él.
// 0.1.1
pub fn disponer(filas: usize, foco: Option<f32>, k: f32, fuerza: f32) -> Vec<(f32, f32)> {
    let (alto, hueco) = (ALTO * k, HUECO * k);
    let paso = alto + hueco;
    let mut arriba = 0.0;
    let mut puestas: Vec<(f32, f32)> = (0..filas)
        .map(|i| {
            let centro = i as f32 * paso + alto / 2.0;
            let s = foco.map_or(1.0, |y| escala(y - centro, k, fuerza));
            let fila = (arriba, s);
            arriba += alto * s + hueco;
            fila
        })
        .collect();
    let Some(y) = foco else {
        return puestas;
    };
    let i = ((y / paso).floor().max(0.0) as usize).min(filas.saturating_sub(1));
    let Some(&(arriba_i, s)) = puestas.get(i) else {
        return puestas;
    };
    let dentro = y - i as f32 * paso;
    let llevado = if dentro < 0.0 {
        dentro
    } else if dentro <= alto {
        arriba_i + dentro * s
    } else {
        arriba_i + alto * s + (dentro - alto)
    };
    let corrimiento = y - llevado;
    for fila in &mut puestas {
        fila.0 += corrimiento;
    }
    puestas
}

/// De 0 a 1, cuánto de la lente le toca a una fila con esa escala.
// 0.1.1
pub fn cercania(escala: f32) -> f32 {
    ((escala - 1.0) / AUMENTO).clamp(0.0, 1.0)
}

/// Qué fila en reposo cae bajo `y`, sin contar los huecos.
// 0.1.1
pub fn fila_en(filas: usize, y: f32, k: f32) -> Option<usize> {
    let paso = (ALTO + HUECO) * k;
    let i = (y / paso).floor();
    (y >= 0.0 && (i as usize) < filas && y - i * paso <= ALTO * k).then_some(i as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_puntero_la_lista_descansa() {
        let filas = disponer(3, None, 1.0, 1.0);
        assert_eq!(filas[1], (ALTO + HUECO, 1.0));
    }

    #[test]
    fn la_fila_señalada_es_la_mayor_y_las_vecinas_crecen_igual() {
        let centro = (ALTO + HUECO) * 2.0 + ALTO / 2.0;
        let filas = disponer(5, Some(centro), 1.0, 1.0);
        assert!((filas[2].1 - (1.0 + AUMENTO)).abs() < 1e-4);
        assert!((filas[1].1 - filas[3].1).abs() < 1e-4);
        assert!(filas[1].1 > 1.0 && filas[1].1 < filas[2].1);
        assert_eq!(filas[0].1, 1.0);
        assert_eq!(filas[4].1, 1.0);
    }

    #[test]
    fn lo_que_hay_bajo_el_puntero_no_se_mueve() {
        for y in [0.0, 10.0, ALTO, ALTO + 6.0, 150.0, 226.0] {
            let filas = disponer(4, Some(y), 1.0, 1.0);
            match fila_en(4, y, 1.0) {
                Some(i) => {
                    let (arriba, s) = filas[i];
                    assert!(arriba <= y + 1e-3 && y <= arriba + ALTO * s + 1e-3, "y={y}");
                }
                None => {
                    let i = (y / (ALTO + HUECO)) as usize;
                    assert!(filas[i].0 + ALTO * filas[i].1 <= y + 1e-3, "y={y}");
                    assert!(filas[i + 1].0 >= y - 1e-3, "y={y}");
                }
            }
        }
    }

    #[test]
    fn sin_fuerza_no_hay_lente() {
        let filas = disponer(3, Some(40.0), 1.0, 0.0);
        assert!(filas.iter().all(|(_, s)| *s == 1.0));
        assert_eq!(filas[0].0, 0.0);
    }
}
