//! `pam_bookos_huella.so`: contraseña **o** huella, lo primero que llegue.
//!
//! Existe por greetd. greetd lleva una sola conversación PAM, y PAM recorre la
//! pila en orden: con `pam_fprintd` delante (lo que pone authselect en
//! `system-auth`) la contraseña no se podía ni escribir hasta que la huella
//! acertaba o se agotaba su tiempo. El bloqueo no lo sufre porque el
//! compositor corre como el usuario y lanza dos servicios a la vez; el greeter
//! corre como `bookos-greeter` y no puede comprobar la contraseña de nadie.
//!
//! Aquí las dos van en paralelo:
//!
//! - Un hilo verifica la huella con fprintd por D-Bus, los mismos pasos que
//!   `pam_fprintd` (`Claim`, `VerifyStart`, `VerifyStatus`, `Release`).
//! - El hilo principal pide la contraseña por la conversación, que bloquea
//!   hasta que el greeter conteste.
//!
//! Una conversación PAM no se puede interrumpir desde dentro. Cuando la huella
//! casa, el hilo lo deja escrito en [`ESTADO`] y el greeter, que lo vigila,
//! contesta vacío para soltarla. Esa respuesta vacía no concede nada: lo que
//! decide es lo que vio este módulo en fprintd.
//!
//! Si llega una contraseña, no se comprueba aquí: se deja en `PAM_AUTHTOK` y se
//! devuelve `PAM_IGNORE` para que la compruebe `pam_unix.so use_first_pass`,
//! que es quien sabe hacerlo (shadow, yescrypt, demoras tras un fallo).
//!
//! Pila prevista, en `session/greeter/bookos-greetd.pam`:
//!
//! ```text
//! auth [success=done ignore=ignore default=die] pam_bookos_huella.so
//! auth required pam_unix.so use_first_pass
//! ```

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

// Valores de `_pam_types.h` de Linux-PAM. No hay cabeceras instaladas: el
// programa de `examples/probar.rs` comprueba con `pam_strerror` que cada
// código dice lo que se espera de él.
pub const PAM_SUCCESS: c_int = 0;
pub const PAM_SYSTEM_ERR: c_int = 4;
pub const PAM_AUTH_ERR: c_int = 7;
pub const PAM_CONV_ERR: c_int = 19;
pub const PAM_IGNORE: c_int = 25;
const PAM_CONV: c_int = 5;
const PAM_AUTHTOK: c_int = 6;
const PAM_PROMPT_ECHO_OFF: c_int = 1;

/// Dónde se deja lo que va pasando con la huella, para el greeter. Se cambia
/// con el argumento `estado=` del módulo (lo usa la prueba, que no es root).
pub const ESTADO: &str = "/run/bookos-huella/estado";

/// Lo que se le pregunta al greeter. Lo compara para traducirlo: el módulo no
/// sabe el idioma de quien va a entrar.
pub const PREGUNTA: &str = "Contraseña o huella";

/// Los mismos intentos que `pam_fprintd` (`max-tries=3`). Agotados, la huella
/// se calla y queda la contraseña.
const INTENTOS: u32 = 3;

const ESPERANDO: u8 = 0;
const COINCIDE: u8 = 1;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type Conversar = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<Conversar>,
    appdata_ptr: *mut c_void,
}

// Enlazado a `libpam.so.0` por su nombre exacto (`+verbatim`): así no hace
// falta el `libpam.so` de `pam-devel`, y el módulo carga aunque quien lo use
// haya abierto libpam con `dlopen` local (lo hace el compositor). Sin esto,
// medido con `examples/probar.rs`: «undefined symbol: pam_get_user». Es lo que
// hacen los módulos de Linux-PAM, que llevan libpam en su `DT_NEEDED`.
#[link(name = "libpam.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn pam_get_user(pamh: *mut c_void, user: *mut *const c_char, prompt: *const c_char) -> c_int;
    fn pam_get_item(pamh: *const c_void, item: c_int, valor: *mut *const c_void) -> c_int;
    fn pam_set_item(pamh: *mut c_void, item: c_int, valor: *const c_void) -> c_int;
}

/// # Safety
/// La llama libpam con un `pamh` válido y `argv` de `argc` cadenas.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_authenticate(
    pamh: *mut c_void,
    _flags: c_int,
    argc: c_int,
    argv: *const *const c_char,
) -> c_int {
    // Un pánico no puede cruzar a C: tumbaría el worker de greetd.
    std::panic::catch_unwind(|| unsafe { autenticar(pamh, argc, argv) }).unwrap_or(PAM_SYSTEM_ERR)
}

/// # Safety
/// Igual que [`pam_sm_authenticate`].
///
/// `PAM_IGNORE` y no `PAM_SUCCESS`: `pam_setcred` recorre la pila `auth` con
/// las mismas reglas, y con `success=done` un éxito aquí cortaba antes de
/// `pam_unix` y de `postlogin`, que son quienes ponen las credenciales.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_setcred(
    _pamh: *mut c_void,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

/// La contraseña recibida, con el nulo final. Se borra al soltarse por
/// cualquier camino, también cuando gana la huella y no se llega a usar.
struct Clave(Vec<u8>);

impl Drop for Clave {
    fn drop(&mut self) {
        unsafe { libc::explicit_bzero(self.0.as_mut_ptr().cast(), self.0.len()) };
    }
}

/// Al registro de autenticación, como los módulos de Linux-PAM. `eprintln!` no
/// vale: en un demonio el descriptor 2 puede estar cerrado y reutilizado por
/// otro fichero.
fn registrar(texto: &str) {
    if let Ok(texto) = std::ffi::CString::new(format!("pam_bookos_huella: {texto}")) {
        unsafe {
            libc::syslog(
                libc::LOG_AUTHPRIV | libc::LOG_WARNING,
                c"%s".as_ptr(),
                texto.as_ptr(),
            )
        };
    }
}

// añadido en la 0.2.0
unsafe fn autenticar(pamh: *mut c_void, argc: c_int, argv: *const *const c_char) -> c_int {
    let estado = unsafe { argumentos(argc, argv) };
    let mut usuario: *const c_char = std::ptr::null();
    if unsafe { pam_get_user(pamh, &mut usuario, std::ptr::null()) } != PAM_SUCCESS
        || usuario.is_null()
    {
        return PAM_AUTH_ERR;
    }
    // Sin `to_string_lossy`: un nombre con bytes no UTF-8 se convertiría en
    // otro, y fprintd verificaría la huella de una cuenta distinta.
    let Ok(usuario) = unsafe { CStr::from_ptr(usuario) }
        .to_str()
        .map(str::to_owned)
    else {
        return PAM_AUTH_ERR;
    };

    let _ = std::fs::remove_file(&estado);
    let resultado = Arc::new(AtomicU8::new(ESPERANDO));
    // Sin bus o sin fprintd queda la contraseña, que es lo que pedía greetd
    // antes de que existiera esto.
    let huella = zbus::blocking::Connection::system().ok().map(|conexion| {
        let (conexion_hilo, resultado_hilo, estado_hilo, usuario_hilo) = (
            conexion.clone(),
            Arc::clone(&resultado),
            estado.clone(),
            usuario.clone(),
        );
        let hilo = std::thread::spawn(move || {
            if let Err(error) =
                vigilar(&conexion_hilo, &usuario_hilo, &resultado_hilo, &estado_hilo)
            {
                registrar(&format!("huella no disponible: {error}"));
            }
        });
        (conexion, hilo)
    });

    let respuesta = unsafe { preguntar(pamh) };

    if let Some((conexion, hilo)) = huella {
        // Solo se cierra la conexión, sin VerifyStop ni Release desde aquí:
        // con fprintd arrancando o colgado esas llamadas bloqueaban la entrada
        // por contraseña. fprintd suelta el lector cuando desaparece quien lo
        // reservó, y el cierre acaba también el iterador de señales en el que
        // espera el hilo.
        let _ = conexion.close();
        let _ = hilo.join();
    }
    let _ = std::fs::remove_file(&estado);

    if resultado.load(Ordering::Acquire) == COINCIDE {
        return PAM_SUCCESS;
    }
    match respuesta {
        Err(codigo) => codigo,
        // Vacía sin huella: o el usuario pulsó Intro sin escribir, o el greeter
        // creyó ver una coincidencia que no hubo. En los dos casos, no.
        Ok(clave) if clave.0.len() <= 1 => PAM_AUTH_ERR,
        Ok(clave) => {
            // pam_set_item guarda su propia copia; la nuestra se borra al
            // salir de aquí.
            let codigo = unsafe { pam_set_item(pamh, PAM_AUTHTOK, clave.0.as_ptr().cast()) };
            if codigo == PAM_SUCCESS {
                PAM_IGNORE
            } else {
                codigo
            }
        }
    }
}

/// `estado=RUTA` cambia [`ESTADO`]; lo demás se ignora, como hacen los módulos
/// de Linux-PAM con lo que no conocen.
// añadido en la 0.2.0
unsafe fn argumentos(argc: c_int, argv: *const *const c_char) -> PathBuf {
    let mut estado = PathBuf::from(ESTADO);
    for i in 0..argc.max(0) as usize {
        let arg = unsafe { *argv.add(i) };
        if arg.is_null() {
            continue;
        }
        if let Some(ruta) = unsafe { CStr::from_ptr(arg) }
            .to_str()
            .ok()
            .and_then(|a| a.strip_prefix("estado="))
        {
            estado = PathBuf::from(ruta);
        }
    }
    estado
}

/// Pide la contraseña. Devuelve los bytes **con** el nulo final, listos para
/// `pam_set_item`.
// añadido en la 0.2.0
unsafe fn preguntar(pamh: *mut c_void) -> Result<Clave, c_int> {
    let mut conv: *const c_void = std::ptr::null();
    if unsafe { pam_get_item(pamh, PAM_CONV, &mut conv) } != PAM_SUCCESS || conv.is_null() {
        return Err(PAM_CONV_ERR);
    }
    let conv = unsafe { &*conv.cast::<PamConv>() };
    let Some(conversar) = conv.conv else {
        return Err(PAM_CONV_ERR);
    };
    let texto = std::ffi::CString::new(PREGUNTA).expect("la pregunta no lleva nulos");
    let mensaje = PamMessage {
        msg_style: PAM_PROMPT_ECHO_OFF,
        msg: texto.as_ptr(),
    };
    let mut lista: *const PamMessage = &mensaje;
    let mut respuestas: *mut PamResponse = std::ptr::null_mut();
    let codigo = unsafe { conversar(1, &mut lista, &mut respuestas, conv.appdata_ptr) };
    if codigo != PAM_SUCCESS || respuestas.is_null() {
        return Err(PAM_CONV_ERR);
    }
    let resp = unsafe { (*respuestas).resp };
    let clave = Clave(if resp.is_null() {
        vec![0]
    } else {
        let bytes = unsafe { CStr::from_ptr(resp) }.to_bytes_with_nul().to_vec();
        unsafe {
            libc::explicit_bzero(resp.cast(), bytes.len());
            libc::free(resp.cast());
        }
        bytes
    });
    // La respuesta la reservó la aplicación con malloc y es nuestra.
    unsafe { libc::free(respuestas.cast()) };
    Ok(clave)
}

/// El dispositivo de huellas por defecto.
// añadido en la 0.2.0
fn dispositivo(
    conexion: &zbus::blocking::Connection,
) -> zbus::Result<zbus::blocking::Proxy<'static>> {
    let gestor = zbus::blocking::Proxy::new(
        conexion,
        "net.reactivated.Fprint",
        "/net/reactivated/Fprint/Manager",
        "net.reactivated.Fprint.Manager",
    )?;
    let ruta: zbus::zvariant::OwnedObjectPath = gestor.call("GetDefaultDevice", &())?;
    zbus::blocking::Proxy::new(
        conexion,
        "net.reactivated.Fprint",
        ruta,
        "net.reactivated.Fprint.Device",
    )
}

/// Verifica la huella hasta que case, se agoten los intentos o se cierre la
/// conexión. Cada cambio queda en `estado` para el greeter.
// añadido en la 0.2.0
fn vigilar(
    conexion: &zbus::blocking::Connection,
    usuario: &str,
    resultado: &AtomicU8,
    estado: &Path,
) -> zbus::Result<()> {
    let lector = dispositivo(conexion)?;
    lector.call::<_, _, ()>("Claim", &(usuario,))?;
    // Antes de `VerifyStart`: con el chip de este portátil el «coincide» llega
    // a los 53 ms del toque y podría adelantarse a la suscripción.
    let senales = lector.receive_signal("VerifyStatus")?;
    // Un usuario sin huellas registradas falla aquí (`NoEnrolledPrints`).
    lector.call::<_, _, ()>("VerifyStart", &("any",))?;
    let mut fallos = 0;
    for senal in senales {
        let (que, terminado): (String, bool) = senal.body().deserialize()?;
        match que.as_str() {
            "verify-match" => {
                // Primero la marca y luego el fichero: el greeter contesta en
                // cuanto lo ve, y la conversación tiene que encontrarla puesta.
                resultado.store(COINCIDE, Ordering::Release);
                // Si no se puede escribir, el greeter no se entera y sigue
                // pidiendo la contraseña; cualquier cosa que conteste entrará,
                // porque la marca ya está puesta. Queda en el registro.
                if let Err(error) = avisar(estado, "coincide") {
                    registrar(&format!("no se pudo avisar de la huella: {error}"));
                }
                break;
            }
            "verify-no-match" => {
                fallos += 1;
                if let Err(error) = avisar(estado, &format!("no-coincide {fallos}")) {
                    registrar(&format!("no se pudo avisar del fallo: {error}"));
                }
                if fallos >= INTENTOS {
                    break;
                }
                // Con `done` fprintd ha parado: hay que volver a empezar.
                if terminado {
                    lector.call::<_, _, ()>("VerifyStop", &())?;
                    lector.call::<_, _, ()>("VerifyStart", &("any",))?;
                }
            }
            // Dedo mal puesto o deslizado corto: fprintd sigue leyendo.
            _ if !terminado => {}
            // Desconectado o error del lector: queda la contraseña.
            _ => break,
        }
    }
    soltar_lector(&lector);
    Ok(())
}

/// Errores ignorados: si ya estaba parado o soltado, fprintd lo dice y da igual.
// añadido en la 0.2.0
fn soltar_lector(lector: &zbus::blocking::Proxy<'_>) {
    let _ = lector.call::<_, _, ()>("VerifyStop", &());
    let _ = lector.call::<_, _, ()>("Release", &());
}

/// Escribe y renombra: el greeter lee el fichero cada 100 ms y nunca debe ver
/// uno a medio escribir.
///
/// Corre como root en /run, así que la carpeta tiene que ser de quien ejecuta
/// el módulo, no un enlace, y sin escritura para nadie más: si otro pudiera
/// escribir en ella, podría poner un enlace en lugar del temporal o un
/// «coincide» falso. Ese «coincide» no abriría la sesión (la marca la pone este
/// módulo), pero cortaría la contraseña a medio escribir.
// añadido en la 0.2.0
fn avisar(estado: &Path, texto: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

    let carpeta = estado
        .parent()
        .ok_or_else(|| std::io::Error::other("ruta de estado sin carpeta"))?;
    match std::fs::DirBuilder::new().mode(0o755).create(carpeta) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => return Err(error),
        _ => {}
    }
    let datos = std::fs::symlink_metadata(carpeta)?;
    if !datos.is_dir() || datos.uid() != unsafe { libc::geteuid() } || datos.mode() & 0o022 != 0 {
        return Err(std::io::Error::other(format!(
            "{} no es una carpeta propia y cerrada",
            carpeta.display()
        )));
    }
    let temporal = estado.with_extension("tmp");
    let _ = std::fs::remove_file(&temporal);
    let mut fichero = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporal)?;
    fichero.write_all(texto.as_bytes())?;
    std::fs::rename(&temporal, estado)
}
