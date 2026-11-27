use std::fs::File;
use std::io::{BufReader, Cursor, Read as _, Seek as _, SeekFrom};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FIRMA: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const FIRMA_BOOKOS: &[u8; 8] = b"BookOS\0\x01";
const MAXIMO_VARIANTE: u32 = 64 * 1024 * 1024;

struct Variante {
    clave: u16,
    posicion: u64,
    longitud: u32,
}

pub(super) struct Variantes {
    por_tema: bool,
    entradas: Vec<Variante>,
}

impl Variantes {
    pub(super) fn abrir(ruta: &Path) -> Option<Self> {
        if !ruta.extension()?.eq_ignore_ascii_case("png") {
            return None;
        }
        let fichero = File::open(ruta).ok()?;
        let tamano = fichero.metadata().ok()?.len();
        Self::leer(&mut BufReader::new(fichero), tamano)
    }

    fn leer(lector: &mut impl std::io::Read, tamano: u64) -> Option<Self> {
        let mut cabecera = [0; 51];
        lector.read_exact(&mut cabecera).ok()?;
        if &cabecera[..8] != FIRMA
            || &cabecera[12..16] != b"IHDR"
            || &cabecera[37..41] != b"boWS"
            || &cabecera[41..49] != FIRMA_BOOKOS
        {
            return None;
        }
        let fin = 41 + u64::from(u32::from_be_bytes(cabecera[33..37].try_into().ok()?));
        if fin + 4 >= tamano {
            return None;
        }

        let (por_tema, cantidad) = match (cabecera[49], cabecera[50]) {
            (0, 2) => (true, 2),
            (1, 4) => (false, 4),
            _ => return None,
        };
        let mut posicion = 51 + 6 * cantidad;
        let mut entradas = Vec::with_capacity(cantidad as usize);
        for _ in 0..cantidad {
            let mut fila = [0; 6];
            lector.read_exact(&mut fila).ok()?;
            let clave = u16::from_be_bytes([fila[0], fila[1]]);
            let longitud = u32::from_be_bytes(fila[2..].try_into().ok()?);
            if longitud > MAXIMO_VARIANTE || clave >= if por_tema { 2 } else { 1440 } {
                return None;
            }
            entradas.push(Variante {
                clave,
                posicion,
                longitud,
            });
            posicion += u64::from(longitud);
        }
        if posicion != fin || entradas.iter().filter(|fila| fila.longitud == 0).count() != 1 {
            return None;
        }
        entradas.sort_unstable_by_key(|fila| fila.clave);
        if entradas.windows(2).any(|par| par[0].clave == par[1].clave) {
            return None;
        }
        Some(Self { por_tema, entradas })
    }

    pub(super) fn por_tema(&self) -> bool {
        self.por_tema
    }

    pub(super) fn horas(&self) -> Vec<u16> {
        if self.por_tema {
            Vec::new()
        } else {
            self.entradas.iter().map(|fila| fila.clave).collect()
        }
    }

    fn activa(&self, claro: bool, minuto: u16) -> Option<&Variante> {
        if self.por_tema {
            return self.entradas.get(usize::from(!claro));
        }
        self.entradas
            .iter()
            .rfind(|fila| fila.clave <= minuto)
            .or_else(|| self.entradas.last())
    }

    pub(super) fn decodificar(&self, ruta: &Path, claro: bool) -> Option<(Vec<u8>, u32, u32)> {
        let variante = self.activa(claro, minutos_locales())?;
        if variante.longitud == 0 {
            return None;
        }

        let mut archivo = File::open(ruta).ok()?;
        archivo.seek(SeekFrom::Start(variante.posicion)).ok()?;
        let mut datos = vec![0; variante.longitud as usize];
        archivo.read_exact(&mut datos).ok()?;
        let mut lector =
            image::ImageReader::with_format(Cursor::new(datos), image::ImageFormat::Png);
        let mut limites = image::Limits::default();
        limites.max_image_width = Some(7680);
        limites.max_image_height = Some(4320);
        lector.limits(limites);
        let rgba = lector.decode().ok()?.into_rgba8();
        let (w, h) = rgba.dimensions();
        Some((rgba.into_raw(), w, h))
    }
}

fn minutos_locales() -> u16 {
    let (hora, minuto) = bookos_shell::hora_local_ahora();
    u16::from(hora) * 60 + u16::from(minuto)
}

fn minutos_hasta(horas: &[u16], minuto: u16) -> Option<u16> {
    let siguiente = horas.iter().copied().find(|hora| *hora > minuto);
    siguiente
        .or_else(|| horas.first().map(|primera| primera + 1440))
        .map(|hora| hora - minuto)
}

pub(super) fn espera(horas: &[u16]) -> Option<Duration> {
    let segundos = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() % 60;
    let minutos = minutos_hasta(horas, minutos_locales())?;
    Some(Duration::from_secs(u64::from(minutos) * 60 - segundos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_horario_envuelve_la_medianoche_y_elige_la_fase_justo_en_su_hora() {
        let horas = [360, 600, 1080, 1320];
        let variantes = Variantes {
            por_tema: false,
            entradas: horas
                .iter()
                .map(|&clave| Variante {
                    clave,
                    longitud: 0,
                    posicion: 0,
                })
                .collect(),
        };
        assert_eq!(variantes.activa(true, 359).map(|v| v.clave), Some(1320));
        assert_eq!(variantes.activa(false, 360).map(|v| v.clave), Some(360));
        assert_eq!(minutos_hasta(&horas, 1439), Some(361));
        assert_eq!(minutos_hasta(&horas, 360), Some(240));
    }

    #[test]
    fn lee_el_indice_del_png_y_rechaza_un_bloque_truncado() {
        let mut datos = [0u8; 81];
        datos[..8].copy_from_slice(FIRMA);
        datos[12..16].copy_from_slice(b"IHDR");
        datos[33..37].copy_from_slice(&24u32.to_be_bytes());
        datos[37..41].copy_from_slice(b"boWS");
        datos[41..49].copy_from_slice(FIRMA_BOOKOS);
        datos[50] = 2;
        datos[58] = 1;
        datos[59..63].copy_from_slice(&2u32.to_be_bytes());
        let leido = Variantes::leer(&mut datos.as_slice(), datos.len() as u64)
            .expect("índice de dos variantes y un PNG principal");
        assert_eq!(leido.activa(true, 0).map(|v| v.longitud), Some(0));
        assert_eq!(leido.activa(false, 0).map(|v| v.longitud), Some(2));
        assert!(Variantes::leer(&mut datos.as_slice(), 61).is_none());
    }
}
