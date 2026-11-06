//! Prueba el módulo sin ser root ni tocar /etc/pam.d.
//!
//! `pam_start_confdir` (Linux-PAM 1.4) lee el servicio de una carpeta propia,
//! y como usuario normal `pam_unix` comprueba la contraseña **propia** a través
//! de `unix_chkpwd`. La conversación hace lo mismo que el greeter: vigila el
//! fichero de estado y contesta vacío si la huella casa; si no, espera una
//! línea por la entrada estándar.
//!
//! ```text
//! cargo build -p bookos-pam-huella
//! cargo run -p bookos-pam-huella --example probar -- <carpeta-temporal>
//! ```
//!
//! Imprime el código de PAM, cuánto tardó, y si la huella o la contraseña
//! fueron las que decidieron.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

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

struct Datos {
    estado: PathBuf,
    lineas: mpsc::Receiver<String>,
    por_huella: bool,
}

unsafe extern "C" fn conversar(
    cuantos: c_int,
    mensajes: *mut *const PamMessage,
    respuestas: *mut *mut PamResponse,
    datos: *mut c_void,
) -> c_int {
    let datos = unsafe { &mut *datos.cast::<Datos>() };
    let bloque = unsafe {
        libc::calloc(cuantos as usize, std::mem::size_of::<PamResponse>()).cast::<PamResponse>()
    };
    for i in 0..cuantos as usize {
        let mensaje = unsafe { &**mensajes.add(i) };
        let texto = unsafe { CStr::from_ptr(mensaje.msg) }.to_string_lossy();
        println!("PAM pregunta (estilo {}): {texto}", mensaje.msg_style);
        if mensaje.msg_style != 1 {
            continue;
        }
        // Como el greeter: cada 100 ms mira si la huella casó.
        let mut visto = String::new();
        let respuesta = loop {
            if let Ok(ahora) = std::fs::read_to_string(&datos.estado)
                && ahora != visto
            {
                println!("estado de la huella: {ahora}");
                if ahora == "coincide" {
                    datos.por_huella = true;
                    break String::new();
                }
                visto = ahora;
            }
            if let Ok(linea) = datos.lineas.recv_timeout(Duration::from_millis(100)) {
                break linea;
            }
        };
        let copia = CString::new(respuesta).expect("una línea no lleva nulos");
        unsafe { (*bloque.add(i)).resp = libc::strdup(copia.as_ptr()) };
    }
    unsafe { *respuestas = bloque };
    0
}

fn main() {
    let carpeta = PathBuf::from(std::env::args().nth(1).expect("uso: probar <carpeta>"));
    let modulo = std::env::current_exe()
        .expect("ruta del ejemplo")
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("libpam_bookos_huella.so");
    assert!(
        modulo.exists(),
        "compila antes: cargo build -p bookos-pam-huella"
    );
    let estado = carpeta.join("estado");
    std::fs::create_dir_all(&carpeta).expect("carpeta de la prueba");
    std::fs::write(
        carpeta.join("bookos-prueba"),
        format!(
            "auth [success=done ignore=ignore default=die] {} estado={}\n\
             auth required pam_unix.so use_first_pass\n",
            modulo.display(),
            estado.display()
        ),
    )
    .expect("servicio de la prueba");
    // Con `confdir` PAM exige también `other`, aunque no se use: sin él,
    // `pam_start_confdir` devuelve PAM_ABORT («no default config other»).
    std::fs::write(carpeta.join("other"), "auth required pam_deny.so\n")
        .expect("servicio other de la prueba");

    let (envio, lineas) = mpsc::channel();
    std::thread::spawn(move || {
        for linea in std::io::stdin().lock().lines().map_while(Result::ok) {
            let _ = envio.send(linea);
        }
    });

    unsafe {
        let pam = libloading::Library::new("libpam.so.0").expect("libpam");
        type Start = unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const PamConv,
            *const c_char,
            *mut *mut c_void,
        ) -> c_int;
        type Auth = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type StrError = unsafe extern "C" fn(*mut c_void, c_int) -> *const c_char;
        let start: libloading::Symbol<Start> = pam.get(b"pam_start_confdir\0").expect("1.4");
        let auth: libloading::Symbol<Auth> = pam.get(b"pam_authenticate\0").expect("auth");
        let end: libloading::Symbol<Auth> = pam.get(b"pam_end\0").expect("end");
        let strerror: libloading::Symbol<StrError> = pam.get(b"pam_strerror\0").expect("str");

        let mut datos = Datos {
            estado,
            lineas,
            por_huella: false,
        };
        let conv = PamConv {
            conv: Some(conversar),
            appdata_ptr: (&raw mut datos).cast(),
        };
        let usuario = CString::new(std::env::var("USER").expect("USER")).expect("usuario");
        let servicio = CString::new("bookos-prueba").expect("servicio");
        let dir = CString::new(carpeta.to_str().expect("ruta UTF-8")).expect("ruta");
        let mut pamh = std::ptr::null_mut();
        assert_eq!(
            start(
                servicio.as_ptr(),
                usuario.as_ptr(),
                &conv,
                dir.as_ptr(),
                &mut pamh
            ),
            0
        );
        // Los códigos declarados a mano en el módulo, contra libpam.
        for codigo in [
            bookos_pam_huella_codigos::SUCCESS,
            bookos_pam_huella_codigos::SYSTEM_ERR,
            bookos_pam_huella_codigos::AUTH_ERR,
            bookos_pam_huella_codigos::CONV_ERR,
            bookos_pam_huella_codigos::IGNORE,
        ] {
            println!(
                "código {codigo}: {}",
                CStr::from_ptr(strerror(pamh, codigo)).to_string_lossy()
            );
        }
        println!("Pon el dedo o escribe la contraseña y pulsa Intro.");
        let inicio = Instant::now();
        let codigo = auth(pamh, 0);
        println!(
            "pam_authenticate = {codigo} ({}) en {:?}; decidió {}",
            CStr::from_ptr(strerror(pamh, codigo)).to_string_lossy(),
            inicio.elapsed(),
            if datos.por_huella {
                "la huella"
            } else {
                "la contraseña"
            }
        );
        end(pamh, codigo);
    }
}

/// Los mismos que declara `src/lib.rs`. Repetidos: el módulo es solo
/// `cdylib` y un ejemplo no puede enlazar con él.
mod bookos_pam_huella_codigos {
    pub const SUCCESS: i32 = 0;
    pub const SYSTEM_ERR: i32 = 4;
    pub const AUTH_ERR: i32 = 7;
    pub const CONV_ERR: i32 = 19;
    pub const IGNORE: i32 = 25;
}
