//! Cuánto cuesta repintar el conmutador de ventanas (Meta+Tab y tres dedos
//! arriba) a la resolución del portátil: 2880 px de ancho a escala 1,75.
use bookos_shell::conmutador::{Entrada, Modo};
use bookos_shell::{Config, Shell};
use std::time::Instant;

fn main() {
    let mut shell = Shell::con_config(1646, 1.75, Config::default());
    let entradas = |n| {
        (0..n)
            .map(|i| Entrada {
                nombre: format!("Ventana {i}"),
                app: String::new(),
                icono: None,
            })
            .collect()
    };
    for n in [3, 6, 10] {
        assert!(shell.abrir_conmutador(Modo::Ventanas, entradas(n), (1646.0, 1029.0)));
        let (w, h) = shell.conmutador_buffer_size().expect("conmutador abierto");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_conmutador(&mut buf);
        let vueltas = 40;
        let t = Instant::now();
        for i in 0..vueltas {
            shell.conmutador_elegir(i % n);
            shell.draw_conmutador(&mut buf);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / vueltas as f64;
        println!("{n:>2} ventanas, tarjeta {w}×{h}: {ms:.2} ms por repintado entero");

        // Lo que se paga ahora al cambiar de celda: solo el rótulo. El recuadro
        // se pinta una vez al abrir y luego solo se mueve.
        let r = shell.conmutador_rotulo().expect("modo ventanas");
        let lw = bookos_shell::a_pixel_entero(r.width, 1.75);
        let lh = bookos_shell::a_pixel_entero(r.height, 1.75);
        let (fw, fh) = ((lw * 1.75).round() as usize, (lh * 1.75).round() as usize);
        let mut rotulo = vec![0u8; fw * fh * 4];
        let t = Instant::now();
        for i in 0..vueltas {
            shell.conmutador_elegir(i % n);
            shell.draw_conmutador_rotulo(&mut rotulo, lw, lh);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / vueltas as f64;
        println!("             rótulo {fw}×{fh}: {ms:.2} ms por cambio de celda");
        shell.cancelar_conmutador();
    }
}
