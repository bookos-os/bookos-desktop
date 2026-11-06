fn main() {
    // Linux-PAM hace dlclose de cada módulo en pam_end, pero el hilo «async-io»
    // que arranca zbus (async-io 2.6, driver.rs: `loop` sin salida) no termina
    // nunca y su código está en este .so: al desmapearlo, ese hilo saltaría a
    // memoria liberada. Con nodelete el .so se queda cargado hasta que acaba
    // el proceso.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
}
