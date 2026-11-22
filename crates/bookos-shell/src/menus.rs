//! Qué ofrecen los menús contextuales del escritorio y del dock.
//!
//! La forma de dibujarlos vive en `emergente::menu_dock`; aquí solo se decide
//! qué entradas hay según lo que se haya pulsado. La regla es la del HIG: solo
//! lo útil. Una entrada que no hace nada en ese caso no sale, en vez de salir
//! atenuada.
//!
//! De dónde sale cada cosa (ver la maqueta del menú del escritorio): la fila de
//! botones de arriba y los recientes del dock, de Windows 11; la lista de
//! ventanas, los colores en el menú y «Opciones ›», de macOS.

use crate::Accion;
use crate::emergente::{
    EntradaMenu as Entrada, FilaMenu as Fila, MarcaMenu as Marca, RapidaMenu as Rapida, t,
};
use crate::escritorio::{Acceso, Elemento, Escritorio, NuevoElemento, Orden, Tamano};

/// Cuántas apps se ofrecen en «Abrir con ›»: más que eso ya es un buscador.
const MAX_ABRIR_CON: usize = 5;
/// Cuántos recientes enseña el dock de una app cerrada.
const MAX_RECIENTES: usize = 3;

fn rapida(etiqueta: &str, icono: &'static str, accion: Accion) -> Rapida {
    Rapida {
        etiqueta: etiqueta.to_string(),
        icono,
        accion,
    }
}

fn fila(etiqueta: &str, icono: &'static str, accion: Accion) -> Entrada {
    Entrada::Fila(Fila::new(etiqueta, icono, accion))
}

/// El menú del escritorio para la selección de ahora. `pegable` dice si el
/// portapapeles trae archivos: sin ellos «Pegar» no sale.
// añadido en la 0.2.3
pub(crate) fn escritorio(esc: &Escritorio, pegable: bool) -> Vec<Entrada> {
    let seleccion: Vec<&Elemento> = esc.seleccionados().collect();
    match seleccion.as_slice() {
        [] => fondo(esc, pegable),
        [uno] => elemento(esc, uno),
        varios => muchos(esc, varios),
    }
}

/// Clic en hueco vacío.
///
/// Usa cabecera + botones rápidos igual que el menú de carpeta para mantener el
/// mismo ancho (`ANCHO_RICO`) y jerarquía visual del HIG.
fn fondo(esc: &Escritorio, pegable: bool) -> Vec<Entrada> {
    let visibles = esc.visible();
    let detalle = if visibles {
        plural(
            esc.cuantos(),
            t("elemento", "item"),
            t("elementos", "items"),
        )
    } else {
        t("Iconos ocultos", "Icons hidden").into()
    };
    let mut entradas = vec![Entrada::Cabecera {
        icono: crate::icono::insignia("monitor"),
        nombre: t("Escritorio", "Desktop").into(),
        detalle,
    }];
    if visibles {
        let segunda = if pegable {
            rapida(t("Pegar", "Paste"), "pegar", Accion::EscritorioPegar)
        } else {
            rapida(t("Alinear", "Clean up"), "rejilla", Accion::EscritorioAlinear)
        };
        entradas.push(Entrada::Rapidas(vec![
            rapida(
                t("Carpeta", "Folder"),
                "carpeta-nueva",
                Accion::EscritorioNuevo(NuevoElemento::Carpeta),
            ),
            segunda,
            rapida(
                t("Terminal", "Terminal"),
                "terminal",
                Accion::EscritorioTerminal,
            ),
        ]));
        let crear = [
            (
                t("Carpeta", "Folder"),
                "carpeta-nueva",
                NuevoElemento::Carpeta,
            ),
            (
                t("Documento de texto", "Text document"),
                "editar",
                NuevoElemento::Texto,
            ),
            (
                t("Script de shell", "Shell script"),
                "terminal",
                NuevoElemento::Script,
            ),
            (
                t("Archivo vacío", "Empty file"),
                "documento",
                NuevoElemento::Vacio,
            ),
        ]
        .into_iter()
        .map(|(e, i, tipo)| fila(e, i, Accion::EscritorioNuevo(tipo)))
        .collect();
        entradas.push(Entrada::Fila(Fila::submenu(
            t("Crear", "New"),
            "mas",
            crear,
        )));
    } else {
        entradas.push(Entrada::Rapidas(vec![
            rapida(
                t("Mostrar", "Show"),
                "ojo",
                Accion::EscritorioIconos(true),
            ),
            rapida(
                t("Terminal", "Terminal"),
                "terminal",
                Accion::EscritorioTerminal,
            ),
            rapida(t("Capturar", "Capture"), "seleccion", Accion::AbrirCaptura),
        ]));
    }
    let tamano = esc.tamano();
    let mut ver = vec![Entrada::Titulo(
        t("Tamaño de los iconos", "Icon size").into(),
    )];
    ver.extend(
        [
            (t("Pequeño", "Small"), Tamano::Pequeno),
            (t("Mediano", "Medium"), Tamano::Mediano),
            (t("Grande", "Large"), Tamano::Grande),
        ]
        .into_iter()
        .map(|(e, tm)| {
            Entrada::Fila(Fila::marcada(
                e,
                Marca::Opcion(tamano == tm),
                Accion::EscritorioTamano(tm),
            ))
        }),
    );
    ver.push(Entrada::Divisor);
    ver.push(Entrada::Fila(Fila::marcada(
        t("Mostrar iconos", "Show icons"),
        Marca::Casilla(visibles),
        Accion::EscritorioIconos(!visibles),
    )));
    entradas.push(Entrada::Fila(Fila::submenu(t("Ver", "View"), "ojo", ver)));
    if visibles {
        let actual = esc.orden();
        let mut orden: Vec<Entrada> = [
            (t("Nombre", "Name"), Orden::Nombre),
            (t("Tipo", "Kind"), Orden::Tipo),
            (t("Fecha de modificación", "Date modified"), Orden::Fecha),
            (t("Tamaño", "Size"), Orden::Tamano),
        ]
        .into_iter()
        .map(|(e, o)| {
            Entrada::Fila(Fila::marcada(
                e,
                Marca::Opcion(actual == Some(o)),
                Accion::EscritorioOrdenar(o),
            ))
        })
        .collect();
        orden.push(Entrada::Divisor);
        orden.push(fila(
            t("Quitar huecos", "Clean up"),
            "rejilla",
            Accion::EscritorioAlinear,
        ));
        entradas.push(Entrada::Fila(Fila::submenu(
            t("Ordenar por", "Sort by"),
            "ordenar",
            orden,
        )));
    }
    entradas.push(Entrada::Divisor);
    entradas.push(fila(
        t("Cambiar fondo…", "Change wallpaper…"),
        "fondo",
        Accion::Lanzar("bookos-settings --page fondos".into()),
    ));
    entradas.push(fila(
        t("Ajustes de pantalla", "Display settings"),
        "pantalla",
        Accion::Lanzar("bookos-settings --page pantalla".into()),
    ));
    entradas
}

/// Qué tipo es, en palabras, para la cabecera.
fn tipo_legible(e: &Elemento) -> String {
    let ext = e
        .ruta
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let tipo = match ext.as_str() {
        "pdf" => t("Documento PDF", "PDF document"),
        "txt" | "md" => t("Documento de texto", "Text document"),
        "odt" | "doc" | "docx" | "rtf" => t("Documento", "Document"),
        "ods" | "xls" | "xlsx" | "csv" => t("Hoja de cálculo", "Spreadsheet"),
        "odp" | "ppt" | "pptx" => t("Presentación", "Presentation"),
        "sh" => t("Script de shell", "Shell script"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "heic" => t("Imagen", "Image"),
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" => t("Audio", "Audio"),
        "mp4" | "mkv" | "webm" | "mov" | "avi" => t("Vídeo", "Video"),
        "zip" | "tar" | "gz" | "xz" | "7z" | "rar" | "zst" => t("Archivo comprimido", "Archive"),
        "desktop" => t("Aplicación", "Application"),
        "" => t("Archivo", "File"),
        _ => return format!("{} {}", t("Archivo", "File"), ext.to_uppercase()),
    };
    tipo.to_string()
}

/// `2,4 MB` con coma decimal en castellano. Base 1000 como el explorador y
/// el resto de escritorios Linux de hoy.
pub(crate) fn tamano_legible(bytes: u64) -> String {
    let (valor, unidad) = match bytes {
        b if b < 1_000 => return format!("{b} B"),
        b if b < 1_000_000 => (b as f64 / 1e3, "KB"),
        b if b < 1_000_000_000 => (b as f64 / 1e6, "MB"),
        b => (b as f64 / 1e9, "GB"),
    };
    let texto = if valor < 10.0 {
        format!("{valor:.1}")
    } else {
        format!("{valor:.0}")
    };
    let texto = if t("es", "en") == "es" {
        texto.replace('.', ",")
    } else {
        texto
    };
    // Espacio duro: «9,8» y «MB» no se separan al partir en renglones.
    format!("{texto}\u{a0}{unidad}")
}

/// Cuántos elementos hay dentro de una carpeta, sin ocultos.
fn contenido(ruta: &std::path::Path) -> usize {
    std::fs::read_dir(ruta)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0)
}

fn plural(n: usize, uno: &str, varios: &str) -> String {
    format!("{n} {}", if n == 1 { uno } else { varios })
}

fn cabecera(e: &Elemento, detalle: String) -> Entrada {
    Entrada::Cabecera {
        icono: e.icono().cloned(),
        nombre: e.nombre.clone(),
        detalle,
    }
}

fn papelera(etiqueta: &str) -> Entrada {
    Entrada::Fila(
        Fila::new(etiqueta, "papelera", Accion::EscritorioEliminarSeleccion)
            .atajo("Supr")
            .peligro(),
    )
}

fn fila_informacion() -> Entrada {
    Entrada::Fila(
        Fila::new(
            t("Información", "Get info"),
            "informacion",
            Accion::EscritorioInformacion,
        )
        .atajo("Alt+Intro"),
    )
}

/// «Abrir con ›»: las apps que dicen abrir ese tipo. Sin ninguna, no sale.
fn abrir_con(e: &Elemento) -> Option<Entrada> {
    let mime = crate::apps::mime_de(&e.ruta)?;
    let apps = crate::apps::para_tipo(&mime, MAX_ABRIR_CON);
    if apps.is_empty() {
        return None;
    }
    let sub = apps
        .into_iter()
        .map(|app| {
            Entrada::Fila(Fila {
                icono: None,
                ..Fila::new(app.nombre, "", Accion::EscritorioAbrirCon(app.exec))
            })
        })
        .collect();
    Some(Entrada::Fila(Fila::submenu(
        t("Abrir con", "Open with"),
        "abrir",
        sub,
    )))
}

/// Un solo icono seleccionado.
fn elemento(esc: &Escritorio, e: &Elemento) -> Vec<Entrada> {
    let abrir = Accion::Lanzar(e.orden().to_string());
    match e.acceso {
        // Sin original no hay nada que abrir ni que mirar: solo quitarlo.
        Acceso::Roto => {
            return vec![
                cabecera(e, t("Original no encontrado", "Original not found").into()),
                papelera(t("Quitar el acceso directo", "Remove shortcut")),
            ];
        }
        Acceso::Si => {
            let original = std::fs::read_link(&e.ruta)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_default();
            return vec![
                cabecera(
                    e,
                    format!("{} · {original}", t("Acceso directo", "Shortcut")),
                ),
                fila(t("Abrir", "Open"), "abrir", abrir),
                fila(
                    t("Mostrar el original", "Show original"),
                    "carpeta",
                    Accion::EscritorioMostrarOriginal,
                ),
                fila_informacion(),
                Entrada::Divisor,
                // A la papelera como todo lo demás: lo que se va es el enlace,
                // y si fue sin querer se recupera igual.
                papelera(t("Quitar el acceso directo", "Remove shortcut")),
            ];
        }
        Acceso::No => {}
    }
    let copiar = rapida(t("Copiar", "Copy"), "copiar", Accion::EscritorioCopiar);
    let renombrar = rapida(
        t("Renombrar", "Rename"),
        "editar",
        Accion::EscritorioRenombrar,
    );
    let ext = e
        .ruta
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut entradas = Vec::new();
    if e.ruta.is_dir() {
        entradas.push(cabecera(
            e,
            format!(
                "{} · {}",
                t("Carpeta", "Folder"),
                plural(
                    contenido(&e.ruta),
                    t("elemento", "item"),
                    t("elementos", "items")
                )
            ),
        ));
        entradas.push(Entrada::Rapidas(vec![
            rapida(t("Abrir", "Open"), "abrir", abrir),
            renombrar,
            copiar,
        ]));
        entradas.push(fila(
            t("Abrir en la terminal", "Open in terminal"),
            "terminal",
            Accion::EscritorioTerminal,
        ));
        entradas.push(fila(
            t("Comprimir", "Compress"),
            "comprimir",
            Accion::EscritorioComprimir,
        ));
        entradas.push(fila_informacion());
        entradas.push(Entrada::Divisor);
        entradas.push(Entrada::Colores {
            elegido: esc.color_seleccion().flatten().map(str::to_string),
        });
        entradas.push(Entrada::Divisor);
        entradas.push(papelera(t("Mover a la papelera", "Move to trash")));
        return entradas;
    }
    let peso = std::fs::metadata(&e.ruta).map_or(0, |m| m.len());
    entradas.push(cabecera(
        e,
        format!("{} · {}", tipo_legible(e), tamano_legible(peso)),
    ));
    match ext.as_str() {
        // Un lanzador se abre o se copia; renombrar su fichero no cambia el
        // nombre que se ve, que es el de dentro.
        "desktop" => {
            entradas.push(Entrada::Rapidas(vec![
                rapida(t("Abrir", "Open"), "abrir", abrir),
                copiar,
            ]));
        }
        "sh" => {
            entradas.push(Entrada::Rapidas(vec![
                rapida(
                    t("Ejecutar", "Run"),
                    "terminal",
                    Accion::Lanzar(format!(
                        "konsole --hold -e {}",
                        crate::escritorio::entrecomillar(&e.ruta)
                    )),
                ),
                rapida(
                    t("Editar", "Edit"),
                    "editar",
                    Accion::EscritorioAbrirCon(editor()),
                ),
                copiar,
            ]));
            entradas.extend(abrir_con(e));
            entradas.push(fila_informacion());
        }
        _ => {
            entradas.push(Entrada::Rapidas(vec![
                rapida(t("Abrir", "Open"), "abrir", abrir),
                renombrar,
                copiar,
            ]));
            entradas.push(Entrada::Fila(
                Fila::new(
                    t("Vista rápida", "Quick look"),
                    "ojo",
                    Accion::EscritorioVistaRapida,
                )
                .atajo("Espacio"),
            ));
            entradas.extend(abrir_con(e));
            entradas.push(fila_informacion());
        }
    }
    entradas.push(Entrada::Divisor);
    entradas.push(papelera(t("Mover a la papelera", "Move to trash")));
    entradas
}

/// El editor de texto predeterminado: un script se edita, no se ejecuta, al
/// pulsar «Editar», aunque `xdg-open` lo mande a la terminal.
fn editor() -> String {
    crate::apps::para_tipo("text/plain", 1)
        .into_iter()
        .next()
        .map_or_else(|| "kwrite".to_string(), |a| a.exec)
}

/// Varios seleccionados.
fn muchos(esc: &Escritorio, sel: &[&Elemento]) -> Vec<Entrada> {
    let carpetas = sel.iter().filter(|e| e.ruta.is_dir()).count();
    let archivos = sel.len() - carpetas;
    let detalle = match (carpetas, archivos) {
        (c, 0) => plural(c, t("carpeta", "folder"), t("carpetas", "folders")),
        (0, a) => plural(a, t("archivo", "file"), t("archivos", "files")),
        (c, a) => format!(
            "{} · {}",
            plural(c, t("carpeta", "folder"), t("carpetas", "folders")),
            plural(a, t("archivo", "file"), t("archivos", "files"))
        ),
    };
    let mut entradas = vec![
        Entrada::Cabecera {
            // El del primero: un pictograma de línea blanco se perdía sobre
            // la tarjeta clara (se vio en la previsualización).
            icono: sel.first().and_then(|e| e.icono().cloned()),
            nombre: format!("{} {}", sel.len(), t("elementos", "items")),
            detalle,
        },
        Entrada::Rapidas(vec![
            rapida(t("Copiar", "Copy"), "copiar", Accion::EscritorioCopiar),
            rapida(
                t("Comprimir", "Compress"),
                "comprimir",
                Accion::EscritorioComprimir,
            ),
            rapida(
                t("Agrupar", "Group"),
                "carpeta-nueva",
                Accion::EscritorioAgrupar,
            ),
        ]),
    ];
    // Los colores solo si todo lo seleccionado son carpetas.
    if let Some(elegido) = esc.color_seleccion() {
        entradas.push(Entrada::Colores {
            elegido: elegido.map(str::to_string),
        });
        entradas.push(Entrada::Divisor);
    }
    entradas.push(papelera(&format!(
        "{} {} {}",
        t("Mover", "Move"),
        sel.len(),
        t("a la papelera", "to trash")
    )));
    entradas
}

/// Lo que sabe el dock de la app sobre la que se abre el menú.
// añadido en la 0.2.3
pub struct AppDock<'a> {
    pub app_id: &'a str,
    pub exec: &'a str,
    pub icono: &'a str,
    pub anclada: bool,
    /// Títulos de sus ventanas, en el orden del compositor, y si es la activa.
    pub ventanas: &'a [(String, bool)],
}

/// El menú de un icono del dock, como el de macOS: sus ventanas por título,
/// abrir otra, «Opciones ›» y cerrar. Cerrada, sus recientes y abrir.
// añadido en la 0.2.3
pub(crate) fn dock(app: &AppDock<'_>) -> Vec<Entrada> {
    let opciones = Entrada::Fila(Fila::submenu(
        t("Opciones", "Options"),
        "preferencias",
        vec![
            Entrada::Fila(Fila::marcada(
                t("Mantener en el dock", "Keep in Dock"),
                Marca::Casilla(app.anclada),
                Accion::Anclar {
                    app_id: app.app_id.to_string(),
                    exec: app.exec.to_string(),
                    icono: app.icono.to_string(),
                    fijar: Some(!app.anclada),
                },
            )),
            Entrada::Fila(Fila::marcada(
                t("Abrir al iniciar sesión", "Open at login"),
                Marca::Casilla(crate::apps::abre_al_iniciar(app.app_id)),
                Accion::AbrirAlIniciar {
                    app_id: app.app_id.to_string(),
                    abrir: !crate::apps::abre_al_iniciar(app.app_id),
                },
            )),
        ],
    ));
    let nueva = fila(
        t("Nueva ventana", "New window"),
        "ventana",
        Accion::Lanzar(app.exec.to_string()),
    );
    // Las propias de la app, menos las que repiten «Nueva ventana».
    let propias = crate::apps::acciones(app.app_id)
        .into_iter()
        .filter(|(nombre, _)| !nombre.eq_ignore_ascii_case(t("Nueva ventana", "New window")))
        .map(|(nombre, exec)| fila(&nombre, "ventana-nueva", Accion::Lanzar(exec)));

    if app.ventanas.is_empty() {
        let mut entradas: Vec<Entrada> = crate::apps::recientes(app.exec, MAX_RECIENTES)
            .into_iter()
            .map(|ruta| {
                let nombre = ruta.file_name().map_or_else(
                    || ruta.to_string_lossy().into_owned(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let icono = if ruta.is_dir() {
                    "carpeta"
                } else {
                    "documento"
                };
                let orden = format!("{} {}", app.exec, crate::escritorio::entrecomillar(&ruta));
                fila(&nombre, icono, Accion::Lanzar(orden))
            })
            .collect();
        if !entradas.is_empty() {
            entradas.push(Entrada::Divisor);
        }
        entradas.push(fila(
            t("Abrir", "Open"),
            "abrir",
            Accion::Lanzar(app.exec.to_string()),
        ));
        entradas.extend(propias);
        entradas.push(opciones);
        return entradas;
    }

    let mut entradas = Vec::new();
    // Con una sola ventana la lista no dice nada que no diga el icono.
    if app.ventanas.len() > 1 {
        entradas.extend(
            app.ventanas
                .iter()
                .enumerate()
                .map(|(n, (titulo, activa))| {
                    // Un título de página web puede tener cien letras: se recorta para
                    // que una ventana no haga el menú del ancho de la pantalla.
                    Entrada::Fila(Fila::marcada(
                        crate::recortar_texto(titulo, 300.0, crate::tema::T_CUERPO),
                        Marca::Casilla(*activa),
                        Accion::VentanaDeApp {
                            app_id: app.app_id.to_string(),
                            n,
                        },
                    ))
                }),
        );
        entradas.push(Entrada::Divisor);
    }
    entradas.push(nueva);
    entradas.extend(propias);
    entradas.push(Entrada::Divisor);
    entradas.push(opciones);
    entradas.push(Entrada::Divisor);
    entradas.push(fila(
        if app.ventanas.len() > 1 {
            t("Cerrar todo", "Quit")
        } else {
            t("Cerrar", "Close")
        },
        "cerrar",
        Accion::Cerrar {
            app_id: app.app_id.to_string(),
        },
    ));
    entradas
}

/// «Hoy, 11:32», «Ayer, 18:40» o «3 nov 2026, 10:12», en hora local.
fn fecha_legible(cuando: std::time::SystemTime) -> String {
    let secs = |t: std::time::SystemTime| {
        t.duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    };
    let (h, m, anio, mes, dia) = crate::state::local_tm(secs(cuando));
    let hoy = crate::state::Fecha::hoy();
    let fecha = crate::state::Fecha {
        anio: anio + 1900,
        mes: mes as u32 + 1,
        dia: dia as u32,
    };
    let ayer = crate::state::local_tm(secs(std::time::SystemTime::now()) - 86_400);
    let es_ayer = (ayer.2, ayer.3, ayer.4) == (anio, mes, dia);
    let dia_texto = if fecha == hoy {
        t("Hoy", "Today").to_string()
    } else if es_ayer {
        t("Ayer", "Yesterday").to_string()
    } else {
        const MESES_ES: [&str; 12] = [
            "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
        ];
        const MESES_EN: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let meses = if t("es", "en") == "es" {
            MESES_ES
        } else {
            MESES_EN
        };
        format!(
            "{} {} {}",
            fecha.dia,
            meses[(fecha.mes - 1) as usize],
            fecha.anio
        )
    };
    format!("{dia_texto}, {h:02}:{m:02}")
}

/// La carpeta de una ruta con `~` en vez de la casa.
fn ubicacion(ruta: &std::path::Path) -> String {
    let padre = ruta.parent().unwrap_or(ruta).to_string_lossy().into_owned();
    match std::env::var("HOME") {
        Ok(casa) if padre.starts_with(&casa) => format!("~{}", &padre[casa.len()..]),
        _ => padre,
    }
}

/// La ficha de información de lo seleccionado (uno solo): lo que se consulta
/// de verdad y, en un script, si se ejecuta al abrirlo.
// añadido en la 0.2.3
pub(crate) fn informacion(esc: &Escritorio) -> Option<crate::emergente::dialogo::Dialogo> {
    use crate::emergente::dialogo::{Dato, Dialogo};
    let mut sel = esc.seleccionados();
    let (Some(e), None) = (sel.next(), sel.next()) else {
        return None;
    };
    let meta = std::fs::metadata(&e.ruta).ok();
    let modificado = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(fecha_legible)
        .unwrap_or_default();
    let mut general = Vec::new();
    let subtitulo = match e.acceso {
        Acceso::Si | Acceso::Roto => {
            let original = std::fs::read_link(&e.ruta)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_default();
            general.push(Dato::Texto(t("Original", "Original").into(), original));
            t("Acceso directo", "Shortcut").to_string()
        }
        Acceso::No if e.ruta.is_dir() => {
            general.push(Dato::Texto(
                t("Contiene", "Contains").into(),
                format!(
                    "{} · {}",
                    plural(
                        contenido(&e.ruta),
                        t("elemento", "item"),
                        t("elementos", "items")
                    ),
                    tamano_legible(crate::escritorio::peso(&e.ruta))
                ),
            ));
            t("Carpeta", "Folder").to_string()
        }
        Acceso::No => {
            let peso = meta.as_ref().map_or(0, |m| m.len());
            let mut valor = tamano_legible(peso);
            // De un texto pequeño se cuentan las líneas: es lo que se quiere
            // saber de un script, más que sus bytes.
            if peso < 1_000_000
                && e.ruta
                    .extension()
                    .is_some_and(|x| x == "sh" || x == "txt" || x == "md")
                && let Ok(texto) = std::fs::read_to_string(&e.ruta)
            {
                valor = format!(
                    "{valor} · {}",
                    plural(
                        texto.lines().count(),
                        t("línea", "line"),
                        t("líneas", "lines")
                    )
                );
            }
            general.push(Dato::Texto(t("Tamaño", "Size").into(), valor));
            tipo_legible(e)
        }
    };
    general.push(Dato::Texto(
        t("Ubicación", "Where").into(),
        ubicacion(&e.ruta),
    ));
    general.push(Dato::Texto(t("Modificado", "Modified").into(), modificado));
    let mut grupos = vec![(t("General", "General").to_string(), general)];
    if e.acceso == Acceso::No && e.ruta.extension().is_some_and(|x| x == "sh") {
        use std::os::unix::fs::PermissionsExt;
        let ejecutable = meta.is_some_and(|m| m.permissions().mode() & 0o111 != 0);
        grupos.push((
            t("Al abrirlo", "When opened").to_string(),
            vec![Dato::Interruptor {
                titulo: t("Ejecutar como programa", "Run as a program").into(),
                detalle: t(
                    "Si está apagado, abrirlo lo muestra en el editor de texto en vez de ejecutarlo.",
                    "When off, opening it shows it in the text editor instead of running it.",
                )
                .into(),
                encendido: ejecutable,
                accion: Accion::EscritorioEjecutable(!ejecutable),
            }],
        ));
    } else if e.acceso == Acceso::No
        && !e.ruta.is_dir()
        && let Some(app) = crate::apps::mime_de(&e.ruta)
            .and_then(|m| crate::apps::para_tipo(&m, 1).into_iter().next())
    {
        grupos.push((
            t("Se abre con", "Opens with").to_string(),
            vec![Dato::Texto(t("Aplicación", "App").into(), app.nombre)],
        ));
    }
    Some(Dialogo::informacion(
        e.icono().cloned(),
        &e.nombre,
        &subtitulo,
        grupos,
    ))
}

/// La vista rápida: imágenes y la primera página de un PDF se ven; un texto,
/// sus primeras líneas; lo demás, el icono en grande con su tipo.
// añadido en la 0.2.3
pub(crate) fn vista_rapida(esc: &Escritorio) -> Option<crate::emergente::dialogo::Dialogo> {
    let mut sel = esc.seleccionados();
    let (Some(e), None) = (sel.next(), sel.next()) else {
        return None;
    };
    if e.ruta.is_dir() {
        return None;
    }
    let peso = std::fs::metadata(&e.ruta).map_or(0, |m| m.len());
    let detalle = format!("{} · {}", tipo_legible(e), tamano_legible(peso));
    let ext = e
        .ruta
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let decodificar = |bytes: &[u8]| {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        Some((img.into_raw(), w, h))
    };
    let imagen = match ext.as_str() {
        // Los mismos códecs que ya trae el crate `image` del shell.
        "png" | "jpg" | "jpeg" | "webp" => {
            std::fs::read(&e.ruta).ok().and_then(|b| decodificar(&b))
        }
        // La primera página con `pdftoppm` (poppler), a 720 px de ancho: lo
        // que cabe en el diálogo a escala 2.
        "pdf" => std::process::Command::new("pdftoppm")
            .args(["-png", "-f", "1", "-l", "1", "-scale-to", "720"])
            .arg(&e.ruta)
            .output()
            .ok()
            .filter(|s| s.status.success())
            .and_then(|s| decodificar(&s.stdout)),
        _ => None,
    };
    let texto = match ext.as_str() {
        "txt" | "md" | "sh" | "rs" | "py" | "js" | "json" | "toml" | "csv" | "log" | "conf"
        | "ini"
            if peso < 1_000_000 =>
        {
            std::fs::read_to_string(&e.ruta).ok()
        }
        _ => None,
    };
    Some(crate::emergente::dialogo::Dialogo::vista(
        &e.nombre,
        &detalle,
        imagen,
        texto.as_deref(),
        e.icono().cloned(),
    ))
}
