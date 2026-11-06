//! Autenticación PAM para salir del bloqueo.
//!
//! PAM se ejecuta en un worker: una contraseña incorrecta tarda segundos a
//! propósito y nunca puede bloquear el bucle de frames. `libpam.so.0` se carga
//! dinámicamente para compilar sin exigir las cabeceras de desarrollo.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_CONV_ERR: c_int = 19;
/// `pam_set_item` con esto fija la función de espera tras un fallo.
const PAM_FAIL_DELAY: c_int = 10;
/// «No puedo recoger la información de autenticación». Es lo que contesta
/// `pam_fprintd` en una máquina **sin lector de huellas**, y no es lo mismo que
/// un rechazo: no hay nada que rechazar. Distinguirlo importa porque tratarlo
/// como fallo hacía que el bloqueo se abriera anunciando «no se pudo verificar
/// la huella» en un portátil que nunca ha tenido sensor.
const PAM_AUTHINFO_UNAVAIL: c_int = 9;

/// Qué contestó PAM, con el único matiz que cambia lo que hay que enseñar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Veredicto {
    Correcto,
    /// La identidad no cuela: contraseña incorrecta, o huella que no casa.
    Rechazado,
    /// No se pudo ni preguntar: no hay lector, o el servicio no está.
    NoDisponible,
}

impl Veredicto {
    // 0.1.0
    pub fn correcto(self) -> bool {
        self == Self::Correcto
    }
}

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

struct Secretos {
    usuario: CString,
    contrasena: CString,
    /// Por donde sale cada `PAM_ERROR_MSG` de la conversación. Solo la huella
    /// lo escucha: con `max-tries`, `pam_fprintd` avisa así de cada dedo que no
    /// casa mientras sigue esperando el siguiente.
    avisos: Option<mpsc::Sender<()>>,
}

/// Sobrescribe a ceros y se asegura de que el compilador no lo tire.
///
/// La contraseña pasa por varias copias antes de llegar a PAM, y el compositor
/// es un proceso que vive toda la sesión y puede acabar en un volcado: lo que no
/// se borre queda ahí legible. `write_volatile` es lo que impide que se optimice
/// el bucle, porque escribir algo que nadie vuelve a leer es justo lo que un
/// optimizador está deseando quitar.
// 0.1.0
fn borrar(bytes: &mut [u8]) {
    for b in bytes {
        // SAFETY: el puntero sale de una referencia mutable viva y válida.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

pub struct Comprobacion {
    resultado: Receiver<Veredicto>,
    avisos: Option<Receiver<()>>,
}

impl Comprobacion {
    // 0.1.0
    pub fn lanzar(usuario: &str, contrasena: &str) -> Option<Self> {
        let usuario = usuario.to_string();
        // En `Vec<u8>` y no en `String` porque hay que poder ponerlo a ceros al
        // terminar, y borrar un `String` por dentro obligaría a tocar sus bytes
        // saltándose la garantía de UTF-8.
        let mut contrasena = contrasena.as_bytes().to_vec();
        let (envio, resultado) = mpsc::sync_channel(1);
        let hilo = std::thread::Builder::new()
            .name("bookos-pam".into())
            .spawn(move || {
                let vale = autenticar(&usuario, &contrasena);
                borrar(&mut contrasena);
                let _ = envio.send(vale);
            });
        if hilo.is_err() {
            return None;
        }
        Some(Self {
            resultado,
            avisos: None,
        })
    }

    /// ¿Ha avisado PAM de un intento fallido desde la última vez que se
    /// preguntó? Solo en la huella; en la contraseña siempre `false`.
    // 0.1.0
    pub fn dedo_no_casa(&mut self) -> bool {
        self.avisos
            .as_ref()
            .is_some_and(|r| r.try_iter().count() > 0)
    }

    // 0.1.0
    pub fn resultado(&mut self) -> Option<Veredicto> {
        match self.resultado.try_recv() {
            Ok(vale) => Some(vale),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Veredicto::Rechazado),
        }
    }

    /// Inicia la conversación PAM configurada por la distribución para el
    /// lector de huellas. El compositor nunca recibe ni almacena datos
    /// biométricos: solo recibe el resultado booleano.
    // 0.1.0
    pub fn huella(usuario: &str) -> Option<Self> {
        // Como máximo una conversación con el sensor, incluso al bloquear
        // otra vez antes de que expire el intento anterior.
        static OCUPADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        use std::sync::atomic::Ordering;
        if !std::path::Path::new("/etc/pam.d/bookos-fingerprint").is_file() {
            return None;
        }
        if OCUPADO.swap(true, Ordering::SeqCst) {
            return None;
        }
        struct Liberar;
        impl Drop for Liberar {
            // 0.1.0
            fn drop(&mut self) {
                OCUPADO.store(false, Ordering::SeqCst);
            }
        }
        let liberar = Liberar;
        let usuario = usuario.to_string();
        let (envio, resultado) = mpsc::sync_channel(1);
        let (aviso, avisos) = mpsc::channel();
        std::thread::Builder::new()
            .name("bookos-fingerprint".into())
            .spawn(move || {
                let _liberar = liberar;
                let _ = envio.send(autenticar_servicio(
                    "bookos-fingerprint",
                    &usuario,
                    b"",
                    Some(aviso),
                ));
            })
            .ok()?;
        Some(Self {
            resultado,
            avisos: Some(avisos),
        })
    }
}

// 0.1.0
pub fn usuario() -> String {
    // La identidad autenticada es el UID de la sesión, nunca una variable
    // de entorno ni root como reserva. La variante reentrante sirve también
    // cuando hay trabajadores PAM activos.
    let mut entrada: libc::passwd = unsafe { std::mem::zeroed() };
    let mut resultado = std::ptr::null_mut();
    let mut buffer = vec![0_u8; 65536];
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entrada,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut resultado,
        )
    };
    if status != 0 || resultado.is_null() || entrada.pw_name.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(entrada.pw_name) }
        .to_string_lossy()
        .into_owned()
}

// 0.1.0
fn servicio() -> &'static str {
    if std::path::Path::new("/etc/pam.d/bookos").is_file() {
        "bookos"
    } else {
        // Compatibilidad con instalaciones anteriores. Estas pilas generales
        // pueden incluir pam_fprintd y demorar el intento; el instalador nuevo
        // crea el servicio `bookos`, exclusivo para la contraseña.
        ["system-auth", "common-auth", "login"]
            .into_iter()
            .find(|nombre| std::path::Path::new("/etc/pam.d").join(nombre).is_file())
            .unwrap_or("login")
    }
}

// 0.1.0
// modified 0.1.1 -- acepta el PIN antes de preguntar a PAM
fn autenticar(usuario: &str, contrasena: &[u8]) -> Veredicto {
    if pin_correcto(usuario, contrasena) {
        return Veredicto::Correcto;
    }
    autenticar_servicio(servicio(), usuario, contrasena, None)
}

/// El PIN que pone Ajustes en `/etc/bookos/pin/<usuario>`: hash SHA-512 crypt,
/// de root y legible por el grupo del usuario. Va fuera de PAM a propósito:
/// en la pila `bookos` haría falta `pam_exec` y un ayudante instalado, y el PIN
/// solo debe abrir el bloqueo, no valer para sudo ni polkit. Lo que no son
/// cifras ni se mira: una contraseña normal va directa a PAM.
// 0.1.1
fn pin_correcto(usuario: &str, intento: &[u8]) -> bool {
    if intento.is_empty() || !intento.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Ok(guardado) = std::fs::read_to_string(format!("/etc/bookos/pin/{usuario}")) else {
        return false;
    };
    pin_casa(guardado.trim(), intento)
}

// 0.1.1
fn pin_casa(guardado: &str, intento: &[u8]) -> bool {
    let (Ok(hash), Ok(frase)) = (CString::new(guardado), CString::new(intento)) else {
        return false;
    };
    // libxcrypt: `.so.2` en Fedora, `.so.1` (compat) en Arch y Debian.
    // `crypt_ra` y no `crypt` porque este hilo puede coincidir con el de la
    // huella y `crypt` devuelve un búfer estático compartido.
    let Some(lib) = ["libcrypt.so.2", "libcrypt.so.1"]
        .into_iter()
        .find_map(|n| unsafe { libloading::Library::new(n) }.ok())
    else {
        tracing::error!("no se pudo cargar libcrypt para comprobar el PIN");
        return false;
    };
    type CryptRa = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *mut *mut c_void,
        *mut c_int,
    ) -> *const c_char;
    // SAFETY: firma de `crypt_ra` en crypt.h de libxcrypt; `datos` lo reserva
    // la biblioteca y se libera con `free` cuando ya no se lee el resultado.
    unsafe {
        let Ok(crypt_ra) = lib.get::<CryptRa>(b"crypt_ra\0") else {
            return false;
        };
        let mut datos: *mut c_void = std::ptr::null_mut();
        let mut tamano: c_int = 0;
        let r = crypt_ra(frase.as_ptr(), hash.as_ptr(), &mut datos, &mut tamano);
        // Un fallo devuelve NULL o una cadena que empieza por «*», que nunca
        // coincide con un hash `$6$`.
        let vale = !r.is_null() && CStr::from_ptr(r) == hash.as_c_str();
        libc::free(datos);
        borrar(&mut frase.into_bytes());
        vale
    }
}

// 0.1.0
fn autenticar_servicio(
    nombre: &str,
    usuario: &str,
    contrasena: &[u8],
    avisos: Option<mpsc::Sender<()>>,
) -> Veredicto {
    if usuario.is_empty() {
        return Veredicto::Rechazado;
    }
    let Ok(usuario) = CString::new(usuario) else {
        return Veredicto::Rechazado;
    };
    let Ok(contrasena) = CString::new(contrasena) else {
        return Veredicto::Rechazado;
    };
    let Ok(servicio) = CString::new(nombre) else {
        return Veredicto::Rechazado;
    };

    // SAFETY: la biblioteca permanece viva hasta después de `pam_end`; las
    // firmas son las de la ABI estable de PAM.
    unsafe {
        let Ok(pam) = libloading::Library::new("libpam.so.0") else {
            tracing::error!("no se pudo cargar libpam.so.0");
            return Veredicto::NoDisponible;
        };
        type Start = unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const PamConv,
            *mut *mut c_void,
        ) -> c_int;
        type Authenticate = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type Account = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type End = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type SetItem = unsafe extern "C" fn(*mut c_void, c_int, *const c_void) -> c_int;
        type StrError = unsafe extern "C" fn(*mut c_void, c_int) -> *const c_char;
        let Ok(start) = pam.get::<Start>(b"pam_start\0") else {
            return Veredicto::NoDisponible;
        };
        let Ok(authenticate) = pam.get::<Authenticate>(b"pam_authenticate\0") else {
            return Veredicto::NoDisponible;
        };
        let Ok(account) = pam.get::<Account>(b"pam_acct_mgmt\0") else {
            return Veredicto::NoDisponible;
        };
        let Ok(end) = pam.get::<End>(b"pam_end\0") else {
            return Veredicto::NoDisponible;
        };
        // Para poder decir **por qué** falló. Sin esto, todos los caminos de
        // error son el mismo `false` y «el bloqueo no me deja entrar» no se
        // puede distinguir de «la contraseña estaba mal»: no hay nada en el
        // registro que separe una cosa de la otra.
        let porque = pam.get::<StrError>(b"pam_strerror\0").ok();
        let motivo = |handle: *mut c_void, codigo: c_int| -> String {
            match &porque {
                Some(f) => CStr::from_ptr(f(handle, codigo))
                    .to_string_lossy()
                    .into_owned(),
                None => format!("código {codigo}"),
            }
        };

        let mut secretos = Secretos {
            usuario,
            contrasena,
            avisos,
        };
        let conversacion = PamConv {
            conv: Some(conversar),
            appdata_ptr: (&mut secretos as *mut Secretos).cast(),
        };
        let mut handle: *mut c_void = std::ptr::null_mut();
        let inicio = start(
            servicio.as_ptr(),
            secretos.usuario.as_ptr(),
            &conversacion,
            &mut handle,
        );
        if inicio != PAM_SUCCESS || handle.is_null() {
            tracing::error!(
                servicio = nombre,
                "pam_start falló: {}",
                motivo(handle, inicio)
            );
            return Veredicto::NoDisponible;
        }
        // Medido con contraseña errónea en este portátil: 1,7-2,2 s por
        // intento, de los que la comprobación real (unix_chkpwd + yescrypt)
        // son 41-55 ms. El resto es la espera que pide `pam_unix` antes de
        // contestar un fallo. Con PAM_FAIL_DELAY la espera la gestiona la
        // aplicación, que es para lo que existe: el bloqueo ya tiene su propia
        // demora creciente (`demora_reintento`), así que esta sobraba y solo
        // hacía lento equivocarse una vez al teclear.
        if let Ok(poner) = pam.get::<SetItem>(b"pam_set_item\0") {
            poner(handle, PAM_FAIL_DELAY, sin_demora as *const c_void);
        }
        // Con la duración en la traza: tras dormir el procesador puede volver
        // a 400 MHz (PL1 de RAPL) y el yescrypt de 50 ms pasa a notarse.
        let desde = std::time::Instant::now();
        let autenticacion = authenticate(handle, 0);
        let resultado = if autenticacion == PAM_SUCCESS {
            account(handle, 0)
        } else {
            autenticacion
        };
        // La contraseña no aparece por ningún lado: lo que se registra es el
        // veredicto de PAM, que es exactamente lo que hace falta para saber si
        // el fallo es «no era la contraseña» o «esto ni siquiera pudo
        // preguntar».
        if resultado == PAM_SUCCESS {
            tracing::info!(
                servicio = nombre,
                ms = desde.elapsed().as_millis() as u64,
                "PAM autenticó"
            );
        } else if autenticacion == PAM_AUTHINFO_UNAVAIL {
            tracing::info!(
                servicio = nombre,
                "PAM no pudo preguntar: {}",
                motivo(handle, autenticacion)
            );
        } else if autenticacion != PAM_SUCCESS {
            tracing::info!(
                servicio = nombre,
                "PAM rechazó: {}",
                motivo(handle, autenticacion)
            );
        } else {
            tracing::warn!(
                servicio = nombre,
                "PAM autenticó pero rechazó la cuenta: {}",
                motivo(handle, resultado)
            );
        }
        let _ = end(handle, resultado);
        // Ya no queda nadie que la mire: ni PAM ni `conversar`. La copia que
        // `conversar` le entregó a PAM la libera PAM por su cuenta y no está en
        // nuestra mano; esta sí.
        let mut bytes = secretos.contrasena.into_bytes_with_nul();
        borrar(&mut bytes);
        match resultado {
            PAM_SUCCESS => Veredicto::Correcto,
            // El único que no es un «no»: significa que no había con qué
            // preguntar. Quien llame decide si eso merece un mensaje o si
            // simplemente se pasa al siguiente método.
            PAM_AUTHINFO_UNAVAIL => Veredicto::NoDisponible,
            _ => Veredicto::Rechazado,
        }
    }
}

/// La función que PAM llama en vez de dormir tras un fallo. No hace nada:
/// la demora la pone el bloqueo.
// 0.1.0
unsafe extern "C" fn sin_demora(
    _estado: c_int,
    _microsegundos: std::ffi::c_uint,
    _datos: *mut c_void,
) {
}

// 0.1.0
unsafe extern "C" fn conversar(
    cuantos: c_int,
    mensajes: *mut *const PamMessage,
    respuestas: *mut *mut PamResponse,
    datos: *mut c_void,
) -> c_int {
    if cuantos <= 0 || mensajes.is_null() || respuestas.is_null() || datos.is_null() {
        return PAM_CONV_ERR;
    }
    let secretos = unsafe { &*(datos.cast::<Secretos>()) };
    let bloque = unsafe {
        libc::calloc(cuantos as usize, std::mem::size_of::<PamResponse>()).cast::<PamResponse>()
    };
    if bloque.is_null() {
        return PAM_CONV_ERR;
    }

    for i in 0..cuantos as usize {
        let mensaje = unsafe { *mensajes.add(i) };
        if mensaje.is_null() {
            liberar_respuestas(bloque, i);
            return PAM_CONV_ERR;
        }
        let estilo = unsafe { (*mensaje).msg_style };
        let texto = match estilo {
            PAM_PROMPT_ECHO_OFF => Some(secretos.contrasena.as_c_str()),
            PAM_PROMPT_ECHO_ON => Some(secretos.usuario.as_c_str()),
            PAM_ERROR_MSG => {
                // El texto viene traducido según el idioma del sistema, así
                // que no se compara: se registra para poder leerlo y se avisa.
                let texto = unsafe { (*mensaje).msg };
                if !texto.is_null() {
                    tracing::info!(
                        "PAM avisa: {}",
                        unsafe { CStr::from_ptr(texto) }.to_string_lossy()
                    );
                }
                if let Some(avisos) = &secretos.avisos {
                    let _ = avisos.send(());
                }
                None
            }
            PAM_TEXT_INFO => None,
            _ => {
                liberar_respuestas(bloque, i);
                return PAM_CONV_ERR;
            }
        };
        if let Some(texto) = texto {
            let copia = copiar_cadena(texto);
            if copia.is_null() {
                liberar_respuestas(bloque, i);
                return PAM_CONV_ERR;
            }
            unsafe { (*bloque.add(i)).resp = copia };
        }
    }
    unsafe { *respuestas = bloque };
    PAM_SUCCESS
}

// 0.1.0
fn copiar_cadena(texto: &CStr) -> *mut c_char {
    let bytes = texto.to_bytes_with_nul();
    let destino = unsafe { libc::malloc(bytes.len()).cast::<c_char>() };
    if !destino.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast(), destino, bytes.len()) };
    }
    destino
}

// 0.1.0
fn liberar_respuestas(respuestas: *mut PamResponse, cuantos: usize) {
    for i in 0..cuantos {
        unsafe { libc::free((*respuestas.add(i)).resp.cast()) };
    }
    unsafe { libc::free(respuestas.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_identidad_ausente_nunca_autentica() {
        assert!(!autenticar_servicio("bookos-fingerprint", "", b"", None).correcto());
        assert!(
            !autenticar_servicio("bookos-fingerprint", "usuario\0invalido", b"", None).correcto()
        );
    }

    #[test]
    fn un_worker_desconectado_falla_cerrado() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let mut comprobacion = Comprobacion {
            resultado: rx,
            avisos: None,
        };
        assert_eq!(comprobacion.resultado(), Some(Veredicto::Rechazado));
    }

    #[test]
    fn el_pin_casa_con_un_hash_de_openssl() {
        // `echo 1234 | openssl passwd -6 -stdin`, como lo genera Ajustes.
        let hash = "$6$yxDdl7B1K2j0KxRU$/LIFIhW18mW1RvDLzHR0OLfzhfFhs2.DNHnglX4zcNAmop3IVI3CFDC7kFVPHZVpG0YjuaJUp7I2mXdaAXPVU1";
        assert!(pin_casa(hash, b"1234"));
        assert!(!pin_casa(hash, b"1235"));
        assert!(!pin_casa("", b"1234"));
    }

    #[test]
    fn una_contrasena_falsa_no_pasa() {
        let mut c = Comprobacion::lanzar(&usuario(), "no-es-esta-contrasena-12345")
            .expect("se puede crear el worker PAM");
        for _ in 0..100 {
            if let Some(veredicto) = c.resultado() {
                assert!(
                    !veredicto.correcto(),
                    "una contraseña inventada ha pasado PAM"
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("PAM no contestó en cinco segundos");
    }
}
