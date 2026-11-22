//! Lo que el menú del escritorio hace con archivos: copiar y pegar por el
//! portapapeles del sistema, accesos directos, comprimir, agrupar, abrir en la
//! terminal.
//!
//! ## Portapapeles
//!
//! Copiar pone una selección **propia** con tres tipos: `text/uri-list` (el
//! estándar), `x-special/gnome-copied-files` (dice si es copia o corte, lo leen
//! Nautilus y Dolphin) y texto con las rutas. Pegar lee la selección de quien
//! sea: si es nuestra, los bytes están aquí; si es de un cliente, se le pide
//! por un pipe y se lee en un hilo, porque un cliente puede tardar o no
//! contestar nunca y el compositor no puede quedarse esperando.
//!
//! ## Nada pesado en el bucle
//!
//! Copiar una carpeta o comprimirla puede tardar minutos. Se hace con `cp` y
//! `zip` en un hilo, y al acabar se avisa por un canal de calloop para releer
//! el escritorio en ese momento en vez de esperar al latido del minuto.

use std::path::{Path, PathBuf};

use smithay::reexports::calloop::channel::{self, Event, Sender};

use crate::state::BookosComp;

/// Lo que guarda el compositor entre copiar y pegar.
#[derive(Default)]
pub struct Estado {
    /// Los tipos de la selección del portapapeles de ahora.
    pub tipos: Vec<String>,
    /// Lo que se va a pegar mientras el diálogo pregunta cómo.
    pendiente: Vec<PathBuf>,
    canal: Option<Sender<Aviso>>,
}

/// Lo que llega de los hilos.
enum Aviso {
    /// Lo leído del portapapeles de un cliente.
    Leido { texto: String, gnome: bool },
    /// Un `cp`, `mv` o `zip` terminó: hay que releer el escritorio.
    Hecho,
}

const URIS: &str = "text/uri-list";
const GNOME: &str = "x-special/gnome-copied-files";

/// ¿Hay archivos que pegar?
// añadido en la 0.2.3
pub fn hay_archivos(state: &BookosComp) -> bool {
    state.archivos.tipos.iter().any(|t| t == URIS || t == GNOME)
}

fn canal(state: &mut BookosComp) -> Sender<Aviso> {
    if let Some(tx) = &state.archivos.canal {
        return tx.clone();
    }
    let (tx, rx) = channel::channel::<Aviso>();
    let insertado = state.loop_handle.insert_source(rx, |evento, _, state| {
        if let Event::Msg(aviso) = evento {
            match aviso {
                Aviso::Leido { texto, gnome } => recibido(state, &texto, gnome),
                Aviso::Hecho => releer(state),
            }
        }
    });
    if let Err(err) = insertado {
        tracing::error!("no se pudo registrar el canal de archivos: {err}");
    }
    state.archivos.canal = Some(tx.clone());
    tx
}

fn releer(state: &mut BookosComp) {
    if let Some(shell) = state.shell.as_mut() {
        shell.escritorio_releer_ya();
    }
    state.needs_redraw = true;
}

/// Las rutas de lo seleccionado en el escritorio.
fn seleccion(state: &BookosComp) -> Vec<PathBuf> {
    state
        .shell
        .as_ref()
        .map_or_else(Vec::new, |s| s.escritorio_rutas_seleccionadas())
}

/// `%`-escapa una ruta para una URI `file://`: todo menos lo no reservado y
/// la barra.
fn uri(ruta: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut salida = String::from("file://");
    for &b in ruta.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            salida.push(b as char);
        } else {
            salida.push_str(&format!("%{b:02X}"));
        }
    }
    salida
}

fn desescapar(uri: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let bytes = uri.as_bytes();
    let mut salida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                salida.push(b);
                i += 3;
            }
            (b, _) => {
                salida.push(b);
                i += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(salida))
}

/// Copia lo seleccionado al portapapeles como archivos.
// añadido en la 0.2.3
pub fn copiar(state: &mut BookosComp) {
    let rutas = seleccion(state);
    if rutas.is_empty() {
        return;
    }
    let uris: Vec<String> = rutas.iter().map(|r| uri(r)).collect();
    let texto: Vec<String> = rutas
        .iter()
        .map(|r| r.to_string_lossy().into_owned())
        .collect();
    let tipos = vec![
        (
            URIS.to_string(),
            format!("{}\r\n", uris.join("\r\n")).into_bytes(),
        ),
        (
            GNOME.to_string(),
            format!("copy\n{}", uris.join("\n")).into_bytes(),
        ),
        (
            "text/plain;charset=utf-8".to_string(),
            texto.join("\n").into_bytes(),
        ),
        ("text/plain".to_string(), texto.join("\n").into_bytes()),
    ];
    let nombres = tipos.iter().map(|(t, _)| t.clone()).collect();
    crate::backend::poner_seleccion_con(
        state,
        nombres,
        crate::handlers::Seleccion::PorTipo(std::sync::Arc::new(tipos)),
    );
}

/// La ruta de lo seleccionado, como texto.
// añadido en la 0.2.3
pub fn copiar_ruta(state: &mut BookosComp) {
    let rutas = seleccion(state);
    if rutas.is_empty() {
        return;
    }
    let texto: Vec<String> = rutas
        .iter()
        .map(|r| r.to_string_lossy().into_owned())
        .collect();
    crate::backend::texto_al_portapapeles(state, &texto.join("\n"));
}

/// Pide lo del portapapeles. Si es nuestro se lee al momento; si es de un
/// cliente, en un hilo, y [`recibido`] sigue cuando llegue.
// añadido en la 0.2.3
pub fn pegar(state: &mut BookosComp) {
    use smithay::wayland::selection::data_device::{
        current_data_device_selection_userdata, request_data_device_client_selection,
    };
    let propio =
        current_data_device_selection_userdata(&state.seat).and_then(|datos| match &*datos {
            crate::handlers::Seleccion::PorTipo(tipos) => tipos
                .iter()
                .find(|(t, _)| t == GNOME)
                .map(|(_, b)| String::from_utf8_lossy(b).into_owned()),
            _ => None,
        });
    if let Some(texto) = propio {
        recibido(state, &texto, true);
        return;
    }
    // GNOME primero porque dice si se cortó; si no, la lista a secas.
    let tipo = if state.archivos.tipos.iter().any(|t| t == GNOME) {
        GNOME
    } else {
        URIS
    };
    let Ok((lectura, escritura)) = smithay::reexports::rustix::pipe::pipe() else {
        return;
    };
    if let Err(err) = request_data_device_client_selection(&state.seat, tipo.into(), escritura) {
        tracing::warn!(?err, "no se pudo pedir el portapapeles");
        return;
    }
    let tx = canal(state);
    let gnome = tipo == GNOME;
    let hilo = std::thread::Builder::new()
        .name("pegar".into())
        .spawn(move || {
            use std::io::Read;
            let mut texto = String::new();
            if std::fs::File::from(lectura)
                .read_to_string(&mut texto)
                .is_ok()
            {
                let _ = tx.send(Aviso::Leido { texto, gnome });
            }
        });
    if let Err(err) = hilo {
        tracing::error!("no se pudo lanzar el hilo de pegar: {err}");
    }
}

/// Lo que había en el portapapeles, ya leído. Cortado se mueve sin preguntar
/// —un acceso directo a algo que se está moviendo no tiene sentido—; copiado,
/// se pregunta.
fn recibido(state: &mut BookosComp, texto: &str, gnome: bool) {
    let mut lineas = texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let cortar = gnome && lineas.clone().next() == Some("cut");
    let rutas: Vec<PathBuf> = lineas
        .by_ref()
        .filter_map(|l| l.strip_prefix("file://"))
        .map(desescapar)
        .filter(|r| r.exists())
        .collect();
    if rutas.is_empty() {
        return;
    }
    let Some(destino) = bookos_shell::escritorio::carpeta_escritorio() else {
        return;
    };
    if cortar {
        mover(state, rutas, &destino);
        return;
    }
    // Lo que ya está en el escritorio no tiene sentido como acceso directo:
    // se duplica sin preguntar, como en cualquier explorador.
    if rutas.iter().all(|r| r.parent() == Some(destino.as_path())) {
        poner(state, rutas, &destino, false);
        return;
    }
    state.archivos.pendiente = rutas;
    let pendiente = &state.archivos.pendiente;
    if let Some(shell) = state.shell.as_mut() {
        shell.abrir_pegar(pendiente);
    }
    state.needs_redraw = true;
}

/// La respuesta del diálogo de pegar.
// añadido en la 0.2.3
pub fn pegar_como(state: &mut BookosComp, acceso: bool) {
    let rutas = std::mem::take(&mut state.archivos.pendiente);
    let Some(destino) = bookos_shell::escritorio::carpeta_escritorio() else {
        return;
    };
    poner(state, rutas, &destino, acceso);
}

/// Un nombre libre en `dir`: «Proyectos», «Proyectos (2)»… con el número
/// antes de la extensión, como al crear.
pub(crate) fn libre(dir: &Path, nombre: &str) -> PathBuf {
    let (base, ext) = match nombre.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b.to_string(), format!(".{e}")),
        _ => (nombre.to_string(), String::new()),
    };
    (1..)
        .map(|n| match n {
            1 => dir.join(nombre),
            n => dir.join(format!("{base} ({n}){ext}")),
        })
        .find(|r| !r.exists() && std::fs::symlink_metadata(r).is_err())
        .expect("la secuencia de números es infinita")
}

/// Copias (en un hilo) o accesos directos (al momento: un enlace no tarda).
fn poner(state: &mut BookosComp, rutas: Vec<PathBuf>, destino: &Path, acceso: bool) {
    if acceso {
        for origen in &rutas {
            let Some(nombre) = origen.file_name() else {
                continue;
            };
            let enlace = libre(destino, &nombre.to_string_lossy());
            if let Err(err) = std::os::unix::fs::symlink(origen, &enlace) {
                tracing::warn!(?origen, "no se pudo crear el acceso directo: {err}");
            }
        }
        releer(state);
        return;
    }
    let pares: Vec<(PathBuf, PathBuf)> = rutas
        .into_iter()
        .filter_map(|o| {
            let nombre = o.file_name()?.to_string_lossy().into_owned();
            Some((o, libre(destino, &nombre)))
        })
        .collect();
    en_hilo(state, "copiar", move || {
        for (origen, copia) in pares {
            // `cp -a` conserva permisos y fechas, y copia enlaces como enlaces.
            orden("cp", |c| c.arg("-a").arg("--").arg(&origen).arg(&copia));
        }
    });
}

fn mover(state: &mut BookosComp, rutas: Vec<PathBuf>, destino: &Path) {
    let pares: Vec<(PathBuf, PathBuf)> = rutas
        .into_iter()
        .filter(|o| o.parent() != Some(destino))
        .filter_map(|o| {
            let nombre = o.file_name()?.to_string_lossy().into_owned();
            Some((o, libre(destino, &nombre)))
        })
        .collect();
    en_hilo(state, "mover", move || {
        for (origen, nuevo) in pares {
            // `rename` es instantáneo en el mismo disco; entre discos falla y
            // `mv` hace la copia y el borrado.
            if std::fs::rename(&origen, &nuevo).is_err() {
                orden("mv", |c| c.arg("--").arg(&origen).arg(&nuevo));
            }
        }
    });
}

fn orden(
    programa: &str,
    args: impl FnOnce(&mut std::process::Command) -> &mut std::process::Command,
) {
    let mut c = std::process::Command::new(programa);
    match args(&mut c).status() {
        Ok(s) if s.success() => {}
        Ok(s) => tracing::warn!(programa, ?s, "terminó con error"),
        Err(err) => tracing::warn!(programa, "no se pudo ejecutar: {err}"),
    }
}

fn en_hilo(state: &mut BookosComp, nombre: &str, trabajo: impl FnOnce() + Send + 'static) {
    let tx = canal(state);
    let hilo = std::thread::Builder::new()
        .name(nombre.into())
        .spawn(move || {
            trabajo();
            let _ = tx.send(Aviso::Hecho);
        });
    if let Err(err) = hilo {
        tracing::error!(nombre, "no se pudo lanzar el hilo: {err}");
    }
}

/// Un `.zip` con lo seleccionado, al lado. Con uno, se llama como él; con
/// varios, «Archivo comprimido.zip», como en Windows y macOS.
// añadido en la 0.2.3
pub fn comprimir(state: &mut BookosComp) {
    let rutas = seleccion(state);
    let (Some(primera), Some(dir)) = (rutas.first(), rutas.first().and_then(|r| r.parent())) else {
        return;
    };
    let base = match rutas.len() {
        1 => primera
            .file_stem()
            .map_or_else(|| "Archivo".into(), |s| s.to_string_lossy().into_owned()),
        _ => bookos_shell::t("Archivo comprimido", "Archive").to_string(),
    };
    let zip = libre(dir, &format!("{base}.zip"));
    let dir = dir.to_path_buf();
    let nombres: Vec<std::ffi::OsString> = rutas
        .iter()
        .filter_map(|r| r.file_name().map(|n| n.to_os_string()))
        .collect();
    en_hilo(state, "comprimir", move || {
        // Desde la carpeta y con nombres relativos: si no, el .zip guardaría
        // `/home/…` entero. `-y` guarda los enlaces como enlaces.
        orden("zip", |c| {
            c.current_dir(&dir)
                .arg("-r")
                .arg("-q")
                .arg("-y")
                .arg(&zip)
                .arg("--")
                .args(&nombres)
        });
    });
}

/// Una terminal en la carpeta seleccionada, o en la del escritorio.
///
/// Konsole y no la terminal propia: `bookos-shell` todavía no acepta una
/// carpeta de inicio por la línea de órdenes (arranca siempre en `HOME`).
// añadido en la 0.2.3
pub fn terminal(state: &mut BookosComp) {
    let carpeta = seleccion(state)
        .into_iter()
        .find(|r| r.is_dir())
        .or_else(bookos_shell::escritorio::carpeta_escritorio);
    let Some(carpeta) = carpeta else { return };
    crate::keybinds::lanzar(
        state,
        &format!(
            "konsole --workdir {}",
            bookos_shell::escritorio::entrecomillar(&carpeta)
        ),
    );
}

/// Mueve lo seleccionado a una carpeta nueva y abre el diálogo para nombrarla.
// añadido en la 0.2.3
pub fn agrupar(state: &mut BookosComp) {
    let rutas = seleccion(state);
    let Some(dir) = bookos_shell::escritorio::carpeta_escritorio() else {
        return;
    };
    if rutas.len() < 2 {
        return;
    }
    let carpeta = libre(&dir, bookos_shell::t("Nueva carpeta", "New folder"));
    if let Err(err) = std::fs::create_dir(&carpeta) {
        tracing::warn!("no se pudo crear la carpeta: {err}");
        return;
    }
    for origen in &rutas {
        if let Some(nombre) = origen.file_name()
            && let Err(err) = std::fs::rename(origen, carpeta.join(nombre))
        {
            tracing::warn!(?origen, "no se pudo mover a la carpeta: {err}");
        }
    }
    releer(state);
    let nombre = carpeta
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(shell) = state.shell.as_mut() {
        shell.escritorio_seleccionar_nombre(&nombre);
        shell.abrir_renombrar();
    }
}

/// Abre lo seleccionado con esa orden.
// añadido en la 0.2.3
pub fn abrir_con(state: &mut BookosComp, exec: &str) {
    let rutas: Vec<String> = seleccion(state)
        .iter()
        .map(|r| bookos_shell::escritorio::entrecomillar(r))
        .collect();
    if rutas.is_empty() {
        return;
    }
    crate::keybinds::lanzar(state, &format!("{exec} {}", rutas.join(" ")));
}

/// Abre la carpeta donde está el original de un acceso directo.
// añadido en la 0.2.3
pub fn mostrar_original(state: &mut BookosComp) {
    let Some(enlace) = seleccion(state).into_iter().next() else {
        return;
    };
    let Ok(original) = std::fs::canonicalize(&enlace) else {
        return;
    };
    let Some(padre) = original.parent() else {
        return;
    };
    crate::keybinds::lanzar(
        state,
        &format!(
            "xdg-open {}",
            bookos_shell::escritorio::entrecomillar(padre)
        ),
    );
}

/// Pone o quita el permiso de ejecución al script seleccionado.
// añadido en la 0.2.3
pub fn ejecutable(state: &mut BookosComp, si: bool) {
    use std::os::unix::fs::PermissionsExt;
    for ruta in seleccion(state) {
        let Ok(meta) = std::fs::metadata(&ruta) else {
            continue;
        };
        let mut permisos = meta.permissions();
        let modo = permisos.mode();
        // El de ejecución va donde ya hay lectura: un archivo que solo lee su
        // dueño no pasa a ejecutable para todos.
        let lectura = (modo & 0o444) >> 2;
        permisos.set_mode(if si { modo | lectura } else { modo & !0o111 });
        if let Err(err) = std::fs::set_permissions(&ruta, permisos) {
            tracing::warn!(?ruta, "no se pudo cambiar el permiso: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_uri_va_y_vuelve() {
        let ruta = Path::new("/home/a/Mis cosas/año 50%.pdf");
        let u = uri(ruta);
        assert_eq!(u, "file:///home/a/Mis%20cosas/a%C3%B1o%2050%25.pdf");
        assert_eq!(
            desescapar(u.strip_prefix("file://").expect("prefijo")),
            ruta
        );
    }

    #[test]
    fn el_nombre_libre_numera_antes_de_la_extension() {
        let dir = std::env::temp_dir().join(format!("bookos-libre-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("carpeta temporal");
        std::fs::write(dir.join("informe.pdf"), b"").expect("fichero");
        assert_eq!(libre(&dir, "informe.pdf"), dir.join("informe (2).pdf"));
        assert_eq!(libre(&dir, "otro"), dir.join("otro"));
        std::fs::remove_dir_all(&dir).expect("limpiar");
    }
}
