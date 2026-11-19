//! Frame format: `51 78 CMD 00 LEN_lo LEN_hi DATA… CRC8(DATA) FF`.

pub mod cmd {
    pub const FEED_BACK: u8 = 0xA0;
    pub const FEED: u8 = 0xA1;
    pub const ROW_RAW: u8 = 0xA2;
    pub const STATUS: u8 = 0xA3;
    pub const QUALITY: u8 = 0xA4;
    pub const CONTROL: u8 = 0xA6;
    pub const DEV_INFO: u8 = 0xA8;
    pub const FLOW: u8 = 0xAE;
    pub const ENERGY: u8 = 0xAF;
    pub const DEV_ID: u8 = 0xBB;
    pub const SPEED: u8 = 0xBD;
    pub const MODE: u8 = 0xBE;
    pub const ROW_RLE: u8 = 0xBF;
    pub const LZO_BLOCK: u8 = 0xCE;
    pub const GRAY_BLOCK: u8 = 0xCF;
    pub const CHALLENGE: u8 = 0xD1;
}

const fn crc_table() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u8;
        let mut b = 0;
        while b < 8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
            b += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC: [u8; 256] = crc_table();

pub fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |c, &b| CRC[(c ^ b) as usize])
}

pub fn frame(cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(data.len() + 8);
    f.extend_from_slice(&[0x51, 0x78, cmd, 0x00]);
    f.extend_from_slice(&(data.len() as u16).to_le_bytes());
    f.extend_from_slice(data);
    f.push(crc8(data));
    f.push(0xFF);
    f
}

/// Variant used by "newFormat" models: same frame with a 0x12 prefix.
pub fn frame_prefixed(cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut f = vec![0x12];
    f.extend(frame(cmd, data));
    f
}

pub fn u16le(v: u32) -> [u8; 2] {
    (v.min(0xFFFF) as u16).to_le_bytes()
}

/// Compressed block (LZO rows or gray rows): DATA = raw_len u16, comp_len u16, payload.
pub fn block(cmd: u8, raw_len: usize, payload: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(payload.len() + 4);
    data.extend_from_slice(&(raw_len as u16).to_le_bytes());
    data.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    data.extend_from_slice(payload);
    frame(cmd, &data)
}

pub fn energy(v: u32) -> Vec<u8> {
    frame(cmd::ENERGY, &u16le(v))
}
pub fn speed(v: u32) -> Vec<u8> {
    frame(cmd::SPEED, &[v.min(255) as u8])
}
pub fn feed(lines: u32) -> Vec<u8> {
    frame(cmd::FEED, &u16le(lines))
}
/// Feed with black-mark detection; negative values move the paper back.
pub fn feed_check_black(lines: i32) -> Vec<u8> {
    let c = if lines < 0 { cmd::FEED_BACK } else { cmd::FEED };
    let v = u16le(lines.unsigned_abs());
    frame(c, &[v[0], v[1], 0x11])
}
pub fn dev_info() -> Vec<u8> {
    frame(cmd::DEV_INFO, &[0])
}
pub fn status() -> Vec<u8> {
    frame(cmd::STATUS, &[0])
}
pub fn stop() -> Vec<u8> {
    frame(cmd::CONTROL, &[0x05])
}
/// Label gap detection used before label jobs.
pub fn check_label_location() -> Vec<u8> {
    let mut v = frame(0xAC, &[0x64, 0x00]);
    v.extend(frame(cmd::CONTROL, &[0x10, 0x02]));
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Image = 0,
    Text = 1,
    Tattoo = 2,
    Photo = 3,
}

pub fn mode(m: Mode) -> Vec<u8> {
    frame(cmd::MODE, &[m as u8])
}

/// Print quality / blackening byte (cmd 0xA4), level 1..=7.
pub fn quality(model: &crate::Model, level: u8) -> Vec<u8> {
    let b = if model.a4xii {
        if level >= 6 {
            0x35
        } else if level >= 3 {
            0x33
        } else {
            0x32
        }
    } else if model.core_print {
        match level {
            3 => 0x35,
            2 => 0x33,
            _ => 0x32,
        }
    } else {
        0x33
    };
    frame(cmd::QUALITY, &[b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_app_constants() {
        // f.f46827a, f.f46830d, f.f46839m, stopPrintByte from the app.
        assert_eq!(dev_info(), [0x51, 0x78, 0xA8, 0, 1, 0, 0, 0, 0xFF]);
        assert_eq!(feed(48), [0x51, 0x78, 0xA1, 0, 2, 0, 48, 0, 0xF9, 0xFF]);
        assert_eq!(mode(Mode::Text), [0x51, 0x78, 0xBE, 0, 1, 0, 1, 7, 0xFF]);
        assert_eq!(stop(), [0x51, 0x78, 0xA6, 0, 1, 0, 5, 27, 0xFF]);
        assert_eq!(
            check_label_location(),
            [0x51, 0x78, 0xAC, 0, 2, 0, 100, 0, 0xA1, 0xFF, 0x51, 0x78, 0xA6, 0, 2, 0, 16, 2, 89, 0xFF]
        );
    }
}
