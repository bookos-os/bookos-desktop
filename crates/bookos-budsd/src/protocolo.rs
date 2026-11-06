//! Protocolo SPP de los Galaxy Buds: tramas, estado extendido y preferencias.
//!
//! Movido casi tal cual desde `BookOS-Settings/src-tauri/src/buds.rs`.
//! Derivado de https://github.com/timschneeb/GalaxyBudsClient
//! (ingeniería inversa del protocolo: timschneeb y colaboradores).

use serde::{Deserialize, Serialize};

// ── CRC16-CCITT table (from GalaxyBudsClient/Utils/Crc16.cs) ─────────────────

static CRC16_TABLE: [u32; 256] = [
    0, 4129, 8258, 12387, 16516, 20645, 24774, 28903, 33032, 37161, 41290, 45419, 49548, 53677,
    57806, 61935, 4657, 528, 12915, 8786, 21173, 17044, 29431, 25302, 37689, 33560, 45947, 41818,
    54205, 50076, 62463, 58334, 9314, 13379, 1056, 5121, 25830, 29895, 17572, 21637, 42346, 46411,
    34088, 38153, 58862, 62927, 50604, 54669, 13907, 9842, 5649, 1584, 30423, 26358, 22165, 18100,
    46939, 42874, 38681, 34616, 63455, 59390, 55197, 51132, 18628, 22757, 26758, 30887, 2112, 6241,
    10242, 14371, 51660, 55789, 59790, 63919, 35144, 39273, 43274, 47403, 23285, 19156, 31415,
    27286, 6769, 2640, 14899, 10770, 56317, 52188, 64447, 60318, 39801, 35672, 47931, 43802, 27814,
    31879, 19684, 23749, 11298, 15363, 3168, 7233, 60846, 64911, 52716, 56781, 44330, 48395, 36200,
    40265, 32407, 28342, 24277, 20212, 15891, 11826, 7761, 3696, 65439, 61374, 57309, 53244, 48923,
    44858, 40793, 36728, 37256, 33193, 45514, 41451, 53516, 49453, 61774, 57711, 4224, 161, 12482,
    8419, 20484, 16421, 28742, 24679, 33721, 37784, 41979, 46042, 49981, 54044, 58239, 62302, 689,
    4752, 8947, 13010, 16949, 21012, 25207, 29270, 46570, 42443, 38312, 34185, 62830, 58703, 54572,
    50445, 13538, 9411, 5280, 1153, 29798, 25671, 21540, 17413, 42971, 47098, 34713, 38840, 59231,
    63358, 50973, 55100, 9939, 14066, 1681, 5808, 26199, 30326, 17941, 22068, 55628, 51565, 63758,
    59695, 39368, 35305, 47498, 43435, 22596, 18533, 30726, 26663, 6336, 2273, 14466, 10403, 52093,
    56156, 60223, 64286, 35833, 39896, 43963, 48026, 19061, 23124, 27191, 31254, 2801, 6864, 10931,
    14994, 64814, 60687, 56684, 52557, 48554, 44427, 40424, 36297, 31782, 27655, 23652, 19525,
    15522, 11395, 7392, 3265, 61215, 65342, 53085, 57212, 44955, 49082, 36825, 40952, 28183, 32310,
    20053, 24180, 11923, 16050, 3793, 7920,
];

// 0.1.0
fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u32 = 0;
    for &b in data {
        crc = CRC16_TABLE[((crc >> 8) ^ b as u32) as usize & 0xFF] ^ (crc << 8);
    }
    (crc & 0xFFFF) as u16
}

// ── Message IDs (subset used by BookOS) ──────────────────────────────────────

pub const MSG_MANAGER_INFO: u8 = 136; // MANAGER_INFO — must send on connect
pub const MSG_STATUS_UPDATED: u8 = 96; // STATUS_UPDATED — basic status response
pub const MSG_EXTENDED_STATUS: u8 = 97; // EXTENDED_STATUS_UPDATED — full status
pub const MSG_NOISE_CONTROLS: u8 = 120; // NOISE_CONTROLS — set ANC mode
pub const MSG_EQUALIZER: u8 = 134; // EQUALIZER — set EQ
pub const MSG_LOCK_TOUCHPAD: u8 = 144; // LOCK_TOUCHPAD — lock/unlock touch
pub const MSG_NOISE_REDUCTION_LEVEL: u8 = 131; // NOISE_REDUCTION_LEVEL — ANC Normal/High
pub const MSG_FIT_TEST_START: u8 = 112; // FIT_TEST — start in-ear seal check
pub const MSG_FIT_TEST_RESULT: u8 = 113; // FIT_TEST result (L=byte0, R=byte1: 0=good,1=loose,2=poor)
pub const MSG_FIT_TEST_STOP: u8 = 114; // FIT_TEST stop
pub const MSG_SET_EASY_PAIRING: u8 = 108; // EASY_PAIRING — multipoint switch
pub const MSG_DEBUG_BUILD_INFO_REQ: u8 = 40; // request build info
pub const MSG_DEBUG_BUILD_INFO_RES: u8 = 33; // response msg_id (observed on Buds3 Pro)
pub const MSG_DEBUG_SERIAL_REQ: u8 = 42; // request SN
pub const MSG_DEBUG_SERIAL_RES: u8 = 42;

// ── Packet framing constants ──────────────────────────────────────────────────

const SOM_STD: u8 = 0xFD; // Start-of-message (standard: Buds+/Live/Pro/2+)
const EOM_STD: u8 = 0xDD; // End-of-message (standard)
const SOM_LEG: u8 = 0xFE; // Start-of-message (legacy: original Buds only)
const EOM_LEG: u8 = 0xEE; // End-of-message (legacy)

// ── Packet encoding ───────────────────────────────────────────────────────────

/// Encode a standard (Buds+/Live/Pro/2+) request packet.
/// Wire format: [SOM][size_lo][size_hi|flags][msg_id][payload…][crc_lo][crc_hi][EOM]
/// size = 1(msgId) + payload.len() + 2(CRC)
// 0.1.0
fn encode_std(msg_id: u8, payload: &[u8]) -> Vec<u8> {
    let size = payload.len() + 3;
    let mut crc_data = Vec::with_capacity(1 + payload.len());
    crc_data.push(msg_id);
    crc_data.extend_from_slice(payload);
    let crc = crc16_ccitt(&crc_data);

    let mut pkt = Vec::with_capacity(1 + 2 + 1 + payload.len() + 2 + 1);
    pkt.push(SOM_STD);
    pkt.push((size & 0xFF) as u8);
    pkt.push(((size >> 8) & 0xFF) as u8);
    pkt.push(msg_id);
    pkt.extend_from_slice(payload);
    pkt.push((crc & 0xFF) as u8); // CRC low
    pkt.push(((crc >> 8) & 0xFF) as u8); // CRC high
    pkt.push(EOM_STD);
    pkt
}

/// Encode a legacy (original Buds) request packet.
/// Wire format: [SOM][type=0][size][msg_id][payload…][crc_lo][crc_hi][EOM]
// 0.1.0
fn encode_leg(msg_id: u8, payload: &[u8]) -> Vec<u8> {
    let size = payload.len() + 3;
    let mut crc_data = Vec::with_capacity(1 + payload.len());
    crc_data.push(msg_id);
    crc_data.extend_from_slice(payload);
    let crc = crc16_ccitt(&crc_data);

    let mut pkt = Vec::with_capacity(1 + 1 + 1 + 1 + payload.len() + 2 + 1);
    pkt.push(SOM_LEG);
    pkt.push(0u8); // Type = Request = 0
    pkt.push(size as u8);
    pkt.push(msg_id);
    pkt.extend_from_slice(payload);
    pkt.push((crc & 0xFF) as u8);
    pkt.push(((crc >> 8) & 0xFF) as u8);
    pkt.push(EOM_LEG);
    pkt
}

// 0.1.0
pub fn encode(msg_id: u8, payload: &[u8], legacy: bool) -> Vec<u8> {
    if legacy {
        encode_leg(msg_id, payload)
    } else {
        encode_std(msg_id, payload)
    }
}

// ── Packet decoding ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct SppPacket {
    pub msg_id: u8,
    pub payload: Vec<u8>,
}

/// Read one SPP packet from `buf`, consuming the bytes.
/// Returns None if not enough data yet.
// 0.1.0
fn parse_one_packet(buf: &[u8]) -> Option<(SppPacket, usize)> {
    if buf.is_empty() {
        return None;
    }

    let legacy = buf[0] == SOM_LEG;
    let standard = buf[0] == SOM_STD;
    if !legacy && !standard {
        return None;
    }

    if legacy {
        // [SOM][type][size][id][payload…][crc×2][EOM]
        if buf.len() < 4 {
            return None;
        }
        let size = buf[2] as usize;
        let total = 1 + 1 + 1 + size + 1; // SOM+type+size + (id+payload+crc2) + EOM
        if buf.len() < total {
            return None;
        }
        let msg_id = buf[3];
        let payload_len = size.saturating_sub(3);
        let payload = buf[4..4 + payload_len].to_vec();
        Some((SppPacket { msg_id, payload }, total))
    } else {
        // [SOM][size_lo][size_hi|flags][id][payload…][crc×2][EOM]
        if buf.len() < 4 {
            return None;
        }
        let size = ((buf[1] as usize) | ((buf[2] as usize & 0x3) << 8)) & 0x3FF;
        let total = 1 + 2 + size + 1; // SOM + header_hi_lo + (id+payload+crc2) + EOM
        if buf.len() < total {
            return None;
        }
        let msg_id = buf[3];
        let payload_len = size.saturating_sub(3);
        let payload = buf[4..4 + payload_len].to_vec();
        Some((SppPacket { msg_id, payload }, total))
    }
}

/// Read all parseable packets from the accumulated buffer.
// 0.1.0
pub fn parse_packets(buf: &mut Vec<u8>) -> Vec<SppPacket> {
    let mut out = Vec::new();
    loop {
        // Find the next SOM
        let start = buf.iter().position(|&b| b == SOM_STD || b == SOM_LEG);
        match start {
            None => {
                buf.clear();
                break;
            }
            Some(0) => {}
            Some(n) => {
                buf.drain(..n);
            }
        }
        match parse_one_packet(buf) {
            Some((pkt, consumed)) => {
                out.push(pkt);
                buf.drain(..consumed);
            }
            None => break,
        }
    }
    out
}

// ── Status types (serializable to JSON for JS frontend) ──────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudsStatus {
    pub connected: bool,
    pub battery_l: u8,
    pub battery_r: u8,
    pub battery_case: u8,
    /// 0=Off, 1=ANC, 2=Ambient, 3=Adaptive
    pub anc_mode: u8,
    /// 0=BassBoost, 1=Soft, 2=Dynamic, 3=Clear, 4=TrebleBoost
    pub eq_preset: u8,
    pub eq_enabled: bool,
    pub wearing_l: bool,
    pub wearing_r: bool,
    pub touchpad_locked: bool,
    pub model: String,
    pub error: Option<String>,
    /// Fit test results: 0=good, 1=loose, 2=poor, 255=not tested
    #[serde(default = "fit_default")]
    pub fit_l: u8,
    #[serde(default = "fit_default")]
    pub fit_r: u8,
    /// Firmware / serial info (populated by buds_request_info)
    #[serde(default)]
    pub fw_left: String,
    #[serde(default)]
    pub fw_right: String,
    #[serde(default)]
    pub serial: String,
    /// El servicio está abriendo el canal de control: Ajustes enseña
    /// «Conectando…» en vez del último error.
    #[serde(default)]
    pub connecting: bool,
    /// `battery_case` es el último valor guardado y no uno recién leído:
    /// con los dos auriculares fuera el estuche no informa (manda 0).
    #[serde(default)]
    pub battery_case_guardada: bool,
    /// De qué auriculares es este estado.
    #[serde(default)]
    pub mac: String,
}
// 0.1.0
fn fit_default() -> u8 {
    255
}
impl Default for BudsStatus {
    // 0.1.0
    fn default() -> Self {
        Self {
            connected: false,
            battery_l: 0,
            battery_r: 0,
            battery_case: 0,
            anc_mode: 0,
            eq_preset: 2,
            eq_enabled: false,
            wearing_l: false,
            wearing_r: false,
            touchpad_locked: false,
            model: String::new(),
            error: None,
            fit_l: 255,
            fit_r: 255,
            fw_left: String::new(),
            fw_right: String::new(),
            serial: String::new(),
            connecting: false,
            battery_case_guardada: false,
            mac: String::new(),
        }
    }
}

// ── Status decoding from EXTENDED_STATUS_UPDATED payload ─────────────────────

// 0.1.0
pub fn decode_extended_status(payload: &[u8], model_hint: &str) -> BudsStatus {
    if payload.len() < 4 {
        return BudsStatus {
            connected: true,
            model: model_hint.to_string(),
            ..Default::default()
        };
    }

    // payload[0] = revision, [1] = earType, [2] = battL, [3] = battR
    let battery_l = payload[2];
    let battery_r = payload[3];

    // Detect legacy (original Buds) by revision byte pattern
    let is_legacy = payload.get(4).copied().unwrap_or(0) < 2 && payload.len() < 15;

    let mut status = BudsStatus {
        connected: true,
        battery_l,
        battery_r,
        model: model_hint.to_string(),
        ..Default::default()
    };

    if is_legacy {
        // Original Buds layout: [rev, earType, battL, battR, coupled, mainConn, wearState,
        //                         ambientEnabled, ambientMode, ambientVol, eqEnabled, eqMode, ...]
        status.wearing_l =
            payload.get(6).copied().unwrap_or(0) == 1 || payload.get(6).copied().unwrap_or(0) == 3;
        status.wearing_r =
            payload.get(6).copied().unwrap_or(0) == 2 || payload.get(6).copied().unwrap_or(0) == 3;
        status.eq_enabled = payload.get(10).copied().unwrap_or(0) != 0;
        status.eq_preset = payload.get(11).copied().unwrap_or(0);
    } else {
        // Buds+ and newer layout: [rev, earType, battL, battR, coupled, mainConn, placement, battCase, ...]
        let placement = payload.get(6).copied().unwrap_or(0);
        status.wearing_l = (placement >> 4) == 0; // 0=Wearing
        status.wearing_r = (placement & 0xF) == 0;
        status.battery_case = payload.get(7).copied().unwrap_or(0);

        // ANC mode starts at different offsets per model generation
        // BudsPlus: [8]=ambient_enabled, [9]=ambient_vol, [10]=adj_sync, [11]=eq_mode
        // BudsLive/Pro and newer: [8]=adj_sync, [9]=eq_mode, [10]=touchlock, [11]=touchpad, [12]=noise_mode
        // We detect based on payload length as a heuristic
        if payload.len() >= 22 {
            // Buds Live / Pro / Buds2 and newer — noise_mode at index 12
            let eq_raw = payload.get(9).copied().unwrap_or(0);
            status.eq_enabled = eq_raw != 0;
            status.eq_preset = if eq_raw == 0 {
                2
            } else {
                eq_raw.saturating_sub(1)
            };
            status.touchpad_locked = payload.get(10).copied().unwrap_or(0) != 0;
            let anc_raw = payload.get(12).copied().unwrap_or(0);
            // NoiseControlModes: Off=0, ANC=1, Ambient=2, Adaptive=3
            status.anc_mode = anc_raw;
        } else if payload.len() >= 14 {
            // Buds+: eq_mode at index 11
            let eq_raw = payload.get(11).copied().unwrap_or(0);
            status.eq_enabled = eq_raw != 0;
            status.eq_preset = if eq_raw == 0 {
                2
            } else {
                eq_raw.saturating_sub(1)
            };
        }
    }
    status
}

// ── Persistent per-MAC preferences (EQ, ANC, touchpad, auto-reconnect) ──
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct BudsPrefs {
    pub eq_enabled: bool,
    pub eq_preset: u8,
    pub anc_mode: u8,
    pub touchpad_locked: bool,
    pub auto_reconnect: bool,
    /// Última batería conocida del estuche, para enseñarla aunque ahora no
    /// informe. 0 = nunca se ha leído.
    #[serde(default)]
    pub bateria_estuche: u8,
}

// 0.1.0
fn prefs_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(format!(
        "{}/.config/bookos-buds.json",
        home
    )))
}

// 0.1.0
fn load_all_prefs() -> std::collections::HashMap<String, BudsPrefs> {
    let p = match prefs_path() {
        Some(p) => p,
        None => return Default::default(),
    };
    let txt = std::fs::read_to_string(&p).unwrap_or_default();
    serde_json::from_str(&txt).unwrap_or_default()
}

// 0.1.0
fn save_all_prefs(map: &std::collections::HashMap<String, BudsPrefs>) {
    if let Some(p) = prefs_path()
        && let Err(e) = std::fs::write(
            &p,
            serde_json::to_string_pretty(map).expect("mapa de texto a tipos simples"),
        )
    {
        eprintln!(
            "budsd: no se guardaron las preferencias en {}: {e}",
            p.display()
        );
    }
}

// 0.1.0
pub fn get_prefs(mac: &str) -> BudsPrefs {
    load_all_prefs().get(mac).cloned().unwrap_or_default()
}

// 0.1.0
pub fn update_prefs<F: FnOnce(&mut BudsPrefs)>(mac: &str, f: F) {
    let mut all = load_all_prefs();
    let entry = all.entry(mac.to_string()).or_default();
    f(entry);
    save_all_prefs(&all);
}

/// Vuelca un paquete recibido sobre el estado. Devuelve `true` si era un
/// estado (básico o extendido), que es lo que trae la batería.
///
/// El estado extendido se decodifica entero y sustituye al anterior, pero
/// prueba de ajuste, firmware, número de serie y MAC no vienen en él: antes se
/// perdían, y como el keepalive pide estado cada 5 s, el resultado de la
/// prueba de ajuste se borraba antes de que Ajustes lo leyera.
// 0.1.0
pub fn aplicar(p: &SppPacket, status: &mut BudsStatus) -> bool {
    match p.msg_id {
        MSG_DEBUG_BUILD_INFO_RES if p.payload.len() > 4 => {
            // Cadenas ASCII imprimibles de 6 o más caracteres, separadas por
            // cualquier otro byte.
            let cadenas: Vec<String> = p
                .payload
                .split(|b| !(0x20..=0x7e).contains(b))
                .filter(|t| t.len() >= 6)
                .map(|t| String::from_utf8_lossy(t).into_owned())
                .collect();
            if let Some(s) = cadenas.first() {
                status.fw_left = s.clone();
            }
            if let Some(s) = cadenas.get(1) {
                status.fw_right = s.clone();
            }
            false
        }
        MSG_DEBUG_SERIAL_RES if p.payload.len() > 4 => {
            let s: String = p
                .payload
                .iter()
                .filter(|b| (0x20..=0x7e).contains(*b))
                .map(|&b| b as char)
                .collect();
            if s.len() >= 6 {
                status.serial = s;
            }
            false
        }
        MSG_FIT_TEST_RESULT => {
            status.fit_l = p.payload.first().copied().unwrap_or(255);
            status.fit_r = p.payload.get(1).copied().unwrap_or(255);
            false
        }
        MSG_STATUS_UPDATED => {
            // El básico no lleva el byte de tipo de auricular que tiene el
            // extendido, así que todo va una posición antes:
            //   [revisión, batL, batR, emparejados, conexión, posición, estuche]
            // Leído con el decodificador del extendido, la batería derecha
            // salía del byte «emparejados» (el «1 %» que aparecía de repente)
            // y el estuche quedaba fuera de la trama.
            let byte = |i: usize| p.payload.get(i).copied();
            let (Some(l), Some(r)) = (byte(1), byte(2)) else {
                return false;
            };
            status.battery_l = l;
            status.battery_r = r;
            if let Some(estuche) = byte(6) {
                poner_estuche(status, estuche);
            }
            true
        }
        MSG_EXTENDED_STATUS => {
            let mut nuevo = decode_extended_status(&p.payload, &status.model);
            let estuche = nuevo.battery_case;
            nuevo.battery_case = status.battery_case;
            nuevo.battery_case_guardada = status.battery_case_guardada;
            nuevo.fit_l = status.fit_l;
            nuevo.fit_r = status.fit_r;
            nuevo.fw_left = std::mem::take(&mut status.fw_left);
            nuevo.fw_right = std::mem::take(&mut status.fw_right);
            nuevo.serial = std::mem::take(&mut status.serial);
            nuevo.mac = std::mem::take(&mut status.mac);
            // Después de la MAC: es con la que se guarda.
            poner_estuche(&mut nuevo, estuche);
            *status = nuevo;
            true
        }
        _ => false,
    }
}

/// Un 0 del estuche es «no lo sé» (ningún auricular dentro), no un estuche
/// vacío: se conserva lo anterior. Un valor real se guarda por MAC para
/// enseñarlo la próxima vez, y solo se escribe a disco si cambia.
// 0.1.0
fn poner_estuche(status: &mut BudsStatus, valor: u8) {
    if valor == 0 || valor > 100 {
        return;
    }
    status.battery_case_guardada = false;
    if status.battery_case == valor {
        return;
    }
    status.battery_case = valor;
    if !status.mac.is_empty() {
        update_prefs(&status.mac, |p| p.bateria_estuche = valor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // El extendido, tal cual llegó de unos Buds3 Pro: L 56, R 56, estuche 0
    // (los dos auriculares puestos, el estuche no informa).
    const EXTENDIDO_REAL: [u8; 20] = [
        4, 8, 56, 56, 1, 1, 17, 0, 0, 3, 255, 34, 0, 0, 84, 1, 84, 1, 4, 0,
    ];

    fn paquete(msg_id: u8, payload: &[u8]) -> SppPacket {
        SppPacket {
            msg_id,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn el_basico_va_una_posicion_antes_que_el_extendido() {
        let mut st = BudsStatus::default();
        // [revisión, L, R, emparejados=1, conexión, posición, estuche]
        assert!(aplicar(
            &paquete(MSG_STATUS_UPDATED, &[4, 55, 54, 1, 1, 17, 80]),
            &mut st
        ));
        assert_eq!((st.battery_l, st.battery_r, st.battery_case), (55, 54, 80));
    }

    #[test]
    fn un_estuche_a_cero_conserva_el_ultimo_valor() {
        let mut st = BudsStatus {
            battery_case: 70,
            battery_case_guardada: true,
            ..Default::default()
        };
        assert!(aplicar(
            &paquete(MSG_EXTENDED_STATUS, &EXTENDIDO_REAL),
            &mut st
        ));
        assert_eq!((st.battery_l, st.battery_r), (56, 56));
        assert_eq!(st.battery_case, 70);
        assert!(st.battery_case_guardada, "un 0 no es una lectura nueva");
    }

    #[test]
    fn una_trama_codificada_se_vuelve_a_leer() {
        let mut buf = encode(MSG_NOISE_CONTROLS, &[2], false);
        // Basura delante: el lector tiene que saltar hasta el inicio de trama.
        buf.insert(0, 0x00);
        let p = parse_packets(&mut buf);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].msg_id, MSG_NOISE_CONTROLS);
        assert_eq!(p[0].payload, vec![2]);
        assert!(buf.is_empty());
    }

    #[test]
    fn el_estado_no_borra_la_prueba_de_ajuste() {
        let mut s = BudsStatus {
            mac: "E8:C9".into(),
            ..Default::default()
        };
        aplicar(
            &SppPacket {
                msg_id: MSG_FIT_TEST_RESULT,
                payload: vec![0, 1],
            },
            &mut s,
        );
        let mut extendido = vec![0u8; 22];
        extendido[2] = 80;
        extendido[3] = 75;
        extendido[7] = 40;
        assert!(aplicar(
            &SppPacket {
                msg_id: MSG_EXTENDED_STATUS,
                payload: extendido
            },
            &mut s
        ));
        assert_eq!((s.battery_l, s.battery_r, s.battery_case), (80, 75, 40));
        assert_eq!((s.fit_l, s.fit_r), (0, 1));
        assert_eq!(s.mac, "E8:C9");
    }
}
