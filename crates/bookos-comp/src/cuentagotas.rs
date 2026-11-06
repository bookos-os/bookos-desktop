//! Meta+Alt+C: coger un color de la pantalla.
//!
//! Mientras está puesto, el ratón y el teclado son suyos —como la capa de
//! captura—: el clic que elige el color no puede llegar a la ventana de debajo.
//! El píxel sale por el mismo camino que una captura de un recorte de 1 × 1
//! (ver `backend::servir_captura_propia`), así que es el color **compuesto**,
//! con transparencias y desenfoques ya aplicados: el que se ve.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::pointer::CursorImageStatus;
use smithay::utils::{Buffer, Point, Rectangle, Transform};

use crate::state::BookosComp;

// 0.1.1
// modificado en la 0.2.0 -- sin cruz y con la primera muestra de la lupa
pub fn alternar(state: &mut BookosComp) {
    if state.cuentagotas {
        cancelar(state);
        return;
    }
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        return;
    }
    state.cuentagotas = true;
    // La lupa hace de puntero: su cuadro central es el píxel que se coge. Con
    // la cruz encima tapaba justo ese píxel.
    state.cursor_status = CursorImageStatus::Hidden;
    muestrear(state);
    state.needs_redraw = true;
}

// 0.1.1
// modificado en la 0.2.0 -- suelta la lupa y devuelve el foco del puntero
pub fn cancelar(state: &mut BookosComp) {
    state.cuentagotas = false;
    state.cuentagotas_lupa = None;
    state.cursor_status = CursorImageStatus::default_named();
    // Mientras estaba puesto el puntero se movía sin foco. Sin repetir el
    // movimiento aquí, el foco seguía vacío hasta mover el ratón: la rueda y
    // los clics de justo después no llegaban a ninguna ventana.
    let time = state.start_time.elapsed().as_millis() as u32;
    crate::input::set_pointer(state, state.pointer_location, time);
    state.needs_redraw = true;
}

/// Pide el píxel que hay bajo el puntero. Se sirve al componer el siguiente
/// fotograma.
// 0.1.1
// modificado en la 0.2.0 -- con lupa, el color es el de su cuadro central
pub fn elegir(state: &mut BookosComp) {
    // Lo que enseña la lupa es lo que se entrega: la captura de 1 × 1 redondea
    // de lógicos a físicos por su cuenta y a escala 1,75 podía caer en el
    // píxel de al lado del marcado.
    if let Some(centro) = state.cuentagotas_lupa.as_ref().and_then(|l| l.centro) {
        cancelar(state);
        entregar(state, centro);
        return;
    }
    let punto = state.pointer_location;
    state.captura_pedida = Some(crate::state::CapturaPedida {
        x: punto.x.floor() as i32,
        y: punto.y.floor() as i32,
        ancho: 1,
        alto: 1,
        destino: crate::state::DestinoCaptura::Color,
    });
    cancelar(state);
}

/// Pide los píxeles de alrededor del puntero para la lupa. Se sirven al
/// componer el siguiente fotograma, como cualquier captura propia.
// añadido en la 0.2.0
pub fn muestrear(state: &mut BookosComp) {
    // Una captura de verdad pendiente manda; la lupa se pone al día en el
    // siguiente movimiento.
    if state
        .captura_pedida
        .is_some_and(|p| p.destino != crate::state::DestinoCaptura::Lupa)
    {
        return;
    }
    state.captura_pedida = Some(crate::state::CapturaPedida {
        x: 0,
        y: 0,
        ancho: LADO,
        alto: LADO,
        destino: crate::state::DestinoCaptura::Lupa,
    });
}

/// Píxeles **físicos** de lado que amplía la lupa. Impar, para que haya uno en
/// el centro. Físicos y no lógicos: a escala 1,75 un lógico son casi dos
/// píxeles, y la lupa está para ver el borde de uno solo.
pub const LADO: i32 = 15;
/// Lógicos de cada píxel ampliado: 15 × 8 = 120 de diámetro, como la de la
/// referencia.
const CELDA: f32 = 8.0;
const ANCHO: i32 = 160;
const ALTO: i32 = 178;
/// Dónde cae el puntero dentro de la imagen, en lógicos.
const CENTRO: (f32, f32) = (80.0, 72.0);

/// La lupa ya pintada, lista para la escena.
pub struct Lupa {
    buffer: MemoryRenderBuffer,
    pixeles: (i32, i32),
    escala: f64,
    /// `None` si el puntero está en el borde y el centro cae fuera.
    centro: Option<[u8; 3]>,
}

/// La región de la captura de la lupa: `LADO` píxeles físicos centrados en el
/// puntero.
// añadido en la 0.2.0
pub fn region(state: &BookosComp, escala: f64) -> Rectangle<i32, Buffer> {
    let x = (state.pointer_location.x * escala).floor() as i32;
    let y = (state.pointer_location.y * escala).floor() as i32;
    Rectangle::new((x - LADO / 2, y - LADO / 2).into(), (LADO, LADO).into())
}

/// Los píxeles leídos, `rgba` del tamaño de `leida`, que es lo que quedó de
/// `pedida` dentro de la pantalla.
// añadido en la 0.2.0
pub fn recibir(
    state: &mut BookosComp,
    rgba: &[u8],
    pedida: Rectangle<i32, Buffer>,
    leida: Rectangle<i32, Buffer>,
    escala: f64,
) {
    if !state.cuentagotas {
        return;
    }
    let muestra: Vec<Option<[u8; 3]>> = (0..LADO)
        .flat_map(|fila| (0..LADO).map(move |columna| (columna, fila)))
        .map(|(columna, fila)| {
            let punto: Point<i32, Buffer> = (pedida.loc.x + columna, pedida.loc.y + fila).into();
            leida.contains(punto).then(|| {
                let i =
                    (((punto.y - leida.loc.y) * leida.size.w + punto.x - leida.loc.x) * 4) as usize;
                [rgba[i], rgba[i + 1], rgba[i + 2]]
            })
        })
        .collect();
    let centro = muestra[(LADO * LADO / 2) as usize];
    match pintar(&muestra, centro, escala) {
        Ok(imagen) => {
            let pixeles = (imagen.width() as i32, imagen.height() as i32);
            state.cuentagotas_lupa = Some(Lupa {
                buffer: MemoryRenderBuffer::from_slice(
                    imagen.data(),
                    Fourcc::Abgr8888,
                    pixeles,
                    1,
                    Transform::Normal,
                    None,
                ),
                pixeles,
                escala,
                centro,
            });
        }
        Err(err) => tracing::warn!("no se pudo pintar la lupa: {err}"),
    }
    state.needs_redraw = true;
}

/// La lupa en la escena, centrada en el puntero.
// añadido en la 0.2.0
pub fn elemento(
    state: &BookosComp,
    renderer: &mut GlesRenderer,
    escala: f64,
) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
    let lupa = state.cuentagotas_lupa.as_ref()?;
    // Pintada para otra escala se vería borrosa; la siguiente muestra ya trae
    // la buena.
    if lupa.escala != escala {
        return None;
    }
    let posicion = (
        state.pointer_location.x * escala - f64::from(CENTRO.0) * escala,
        state.pointer_location.y * escala - f64::from(CENTRO.1) * escala,
    );
    let src = Rectangle::from_size((f64::from(lupa.pixeles.0), f64::from(lupa.pixeles.1)).into());
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        posicion,
        &lupa.buffer,
        None,
        Some(src),
        Some((ANCHO, ALTO).into()),
        Kind::Unspecified,
    )
    .ok()
}

/// Rasteriza la lupa: círculo con los píxeles ampliados, rejilla, el central
/// marcado y debajo la etiqueta con la muestra y el hex.
///
/// Los bordes llevan un trazo oscuro además del claro: solo con el aro claro
/// de la referencia, sobre una página blanca la lupa no tenía contorno.
// añadido en la 0.2.0
fn pintar(
    muestra: &[Option<[u8; 3]>],
    centro: Option<[u8; 3]>,
    escala: f64,
) -> anyhow::Result<resvg::tiny_skia::Pixmap> {
    use std::fmt::Write;

    let radio = CELDA * LADO as f32 / 2.0;
    let (cx, cy) = CENTRO;
    let (x0, y0) = (cx - radio, cy - radio);
    let mut celdas = String::new();
    for (i, color) in muestra.iter().enumerate() {
        // Fuera de la pantalla: negro, que es lo que hay más allá del borde.
        let [r, g, b] = color.unwrap_or([0, 0, 0]);
        let (columna, fila) = ((i as i32 % LADO) as f32, (i as i32 / LADO) as f32);
        write!(
            celdas,
            r#"<rect x="{}" y="{}" width="{CELDA}" height="{CELDA}" fill="rgb({r},{g},{b})"/>"#,
            x0 + columna * CELDA,
            y0 + fila * CELDA,
        )?;
    }
    let mut rejilla = String::new();
    for k in 1..LADO {
        let d = k as f32 * CELDA;
        write!(
            rejilla,
            "M{} {y0}V{}M{x0} {}H{}",
            x0 + d,
            y0 + 2.0 * radio,
            y0 + d,
            x0 + 2.0 * radio,
        )?;
    }
    let medio = CELDA / 2.0;
    let (hex, etiqueta) = match centro {
        Some(c) => (hex(c), hex(c)),
        None => ("#000000".to_string(), "—".to_string()),
    };
    let fuente = "Inter, Noto Sans, Cantarell, DejaVu Sans, sans-serif";
    let y_etiqueta = cy + radio + 12.0;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{ANCHO}" height="{ALTO}" viewBox="0 0 {ANCHO} {ALTO}">
<defs>
<clipPath id="c"><circle cx="{cx}" cy="{cy}" r="{radio}"/></clipPath>
<filter id="s" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#000" flood-opacity="0.35"/></filter>
</defs>
<circle cx="{cx}" cy="{cy}" r="{r_aro}" fill="#E6E6E6" filter="url(#s)"/>
<g clip-path="url(#c)" shape-rendering="crispEdges">{celdas}</g>
<path d="{rejilla}" clip-path="url(#c)" stroke="#808080" stroke-opacity="0.3" stroke-width="0.5" fill="none"/>
<circle cx="{cx}" cy="{cy}" r="{radio}" fill="none" stroke="#000" stroke-opacity="0.25" stroke-width="1"/>
<circle cx="{cx}" cy="{cy}" r="{r_aro}" fill="none" stroke="#000" stroke-opacity="0.3" stroke-width="1"/>
<rect x="{cxm}" y="{cym}" width="{CELDA}" height="{CELDA}" fill="none" stroke="#000" stroke-width="2"/>
<rect x="{cxm2}" y="{cym2}" width="{c2}" height="{c2}" fill="none" stroke="#FFF" stroke-width="1"/>
<rect x="4" y="{y_etiqueta}" width="152" height="34" rx="6" fill="#E6E6E6" stroke="#000" stroke-opacity="0.2" stroke-width="1" filter="url(#s)"/>
<rect x="11" y="{y_muestra}" width="20" height="20" fill="{hex}" stroke="#FFF" stroke-width="2"/>
<rect x="9.5" y="{y_muestra2}" width="23" height="23" fill="none" stroke="#000" stroke-opacity="0.35" stroke-width="1"/>
<text x="92" y="{y_texto}" text-anchor="middle" font-family="{fuente}" font-size="16" fill="#1E1E1E">{etiqueta}</text>
</svg>"##,
        r_aro = radio + 3.0,
        cxm = cx - medio,
        cym = cy - medio,
        cxm2 = cx - medio - 1.5,
        cym2 = cy - medio - 1.5,
        c2 = CELDA + 3.0,
        y_muestra = y_etiqueta + 7.0,
        y_muestra2 = y_etiqueta + 5.5,
        y_texto = y_etiqueta + 23.0,
    );
    let arbol = OPCIONES.with(|o| resvg::usvg::Tree::from_str(&svg, o))?;
    let ancho = (f64::from(ANCHO) * escala).round() as u32;
    let alto = (f64::from(ALTO) * escala).round() as u32;
    let mut imagen = resvg::tiny_skia::Pixmap::new(ancho, alto)
        .ok_or_else(|| anyhow::anyhow!("lupa de {ancho} × {alto}"))?;
    resvg::render(
        &arbol,
        resvg::tiny_skia::Transform::from_scale(escala as f32, escala as f32),
        &mut imagen.as_mut(),
    );
    Ok(imagen)
}

thread_local! {
    /// Las fuentes del sistema, cargadas una vez: leerlas en cada movimiento
    /// del ratón sería releer todo `/usr/share/fonts` por fotograma.
    static OPCIONES: resvg::usvg::Options<'static> = {
        let mut opciones = resvg::usvg::Options::default();
        opciones.fontdb_mut().load_system_fonts();
        opciones
    };
}

/// El color ya leído: al portapapeles y un aviso con una muestra.
// 0.1.1
pub fn entregar(state: &mut BookosComp, [r, g, b]: [u8; 3]) {
    let hex = hex([r, g, b]);
    tracing::info!(%hex, "color cogido");
    crate::backend::texto_al_portapapeles(state, &hex);
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    let mut aviso = bookos_shell::notificaciones::Notificacion::nueva(
        0,
        "Cuentagotas".into(),
        format!("{hex} copiado"),
        format!("rgb({r}, {g}, {b})"),
        "",
        false,
    );
    // La muestra hace de icono: el número solo no dice si se cogió el color
    // que se quería o el del borde de al lado.
    aviso.icono = Some(bookos_shell::icono::desde_svg(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="18" fill="{hex}" stroke="#808080" stroke-opacity="0.5" stroke-width="1"/></svg>"##
    )));
    shell.notificar(aviso, 4000);
    state.needs_redraw = true;
}

// 0.1.1
fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn el_hex_va_en_mayusculas_y_con_dos_cifras() {
        assert_eq!(super::hex([10, 79, 255]), "#0A4FFF");
    }
}
