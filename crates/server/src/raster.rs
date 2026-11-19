//! Decoders for the document formats IPP clients send: PWG raster, Apple URF,
//! and plain JPEG/PNG. Every page becomes an 8-bit grayscale image.

use image::{DynamicImage, GrayImage};

fn be32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn luma(px: &[u8], space: Space) -> u8 {
    match space {
        Space::Gray => px[0],
        Space::Black => 255 - px[0],
        Space::Rgb => ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000) as u8,
        Space::Cmyk => {
            let k = 255 - px[3] as u32;
            (((255 - px[0] as u32) * k / 255 * 299 + (255 - px[1] as u32) * k / 255 * 587 + (255 - px[2] as u32) * k / 255 * 114) / 1000) as u8
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Space {
    Gray,
    Black,
    Rgb,
    Cmyk,
}

/// Shared PWG/URF compression: per line a repeat count, then runs of pixels.
struct Page {
    width: usize,
    height: usize,
    bpp: usize,
    space: Space,
    urf: bool,
}

fn decode_page(b: &[u8], mut i: usize, p: &Page) -> Option<(GrayImage, usize)> {
    let bytes_pp = (p.bpp / 8).max(1);
    // 1-bit pages pack 8 pixels per byte: decode bytes, expand later.
    let line_bytes = if p.bpp == 1 { p.width.div_ceil(8) } else { p.width * bytes_pp };
    let unit = if p.bpp == 1 { 1 } else { bytes_pp };
    let white: Vec<u8> = match (p.space, p.bpp) {
        (Space::Black, _) => vec![0; unit],
        (_, 1) => vec![0xFF; unit],
        (Space::Cmyk, _) => vec![0; unit],
        _ => vec![0xFF; unit],
    };
    let mut img = GrayImage::new(p.width as u32, p.height as u32);
    let mut y = 0;
    while y < p.height {
        let repeat = *b.get(i)? as usize + 1;
        i += 1;
        let mut line = Vec::with_capacity(line_bytes);
        while line.len() < line_bytes {
            let c = *b.get(i)?;
            i += 1;
            if c == 0x80 && p.urf {
                // URF: fill the rest of the line with white.
                while line.len() < line_bytes {
                    line.extend_from_slice(&white);
                }
            } else if c < 128 {
                let px = b.get(i..i + unit)?;
                i += unit;
                for _ in 0..=c as usize {
                    line.extend_from_slice(px);
                }
            } else {
                let n = 257 - c as usize;
                let px = b.get(i..i + n * unit)?;
                i += n * unit;
                line.extend_from_slice(px);
            }
        }
        line.truncate(line_bytes);
        for _ in 0..repeat {
            if y >= p.height {
                break;
            }
            for x in 0..p.width {
                let v = if p.bpp == 1 {
                    let bit = (line[x / 8] >> (7 - x % 8)) & 1;
                    // Black colorspace: 1 = ink; gray: 1 = white.
                    match p.space {
                        Space::Black => if bit == 1 { 0 } else { 255 },
                        _ => if bit == 1 { 255 } else { 0 },
                    }
                } else {
                    luma(&line[x * bytes_pp..x * bytes_pp + bytes_pp.max(1)], p.space)
                };
                img.put_pixel(x as u32, y as u32, image::Luma([v]));
            }
            y += 1;
        }
    }
    Some((img, i))
}

pub fn pwg(b: &[u8]) -> Option<Vec<GrayImage>> {
    if b.get(0..4)? != b"RaS2" {
        return None;
    }
    let mut i = 4;
    let mut pages = Vec::new();
    while i + 1796 <= b.len() {
        let h = &b[i..i + 1796];
        let width = be32(h, 372)? as usize;
        let height = be32(h, 376)? as usize;
        let bits_color = be32(h, 384)? as usize;
        let bpp = be32(h, 388)? as usize;
        let cs = be32(h, 400)?;
        let space = match cs {
            3 => Space::Black,
            1 | 19 | 20 => Space::Rgb,
            6 => Space::Cmyk,
            _ => Space::Gray,
        };
        let _ = bits_color;
        if width == 0 || height == 0 || width > 20000 || height > 100_000 {
            return None;
        }
        let (img, next) = decode_page(b, i + 1796, &Page { width, height, bpp, space, urf: false })?;
        pages.push(img);
        i = next;
    }
    Some(pages)
}

pub fn urf(b: &[u8]) -> Option<Vec<GrayImage>> {
    if b.get(0..8)? != b"UNIRAST\0" {
        return None;
    }
    let count = be32(b, 8)? as usize;
    let mut i = 12;
    let mut pages = Vec::new();
    for _ in 0..count.max(1) {
        if i + 32 > b.len() {
            break;
        }
        let h = &b[i..i + 32];
        let bpp = h[0] as usize;
        let space = match h[1] {
            0 | 4 => Space::Gray,
            1 | 3 | 5 => Space::Rgb,
            6 => Space::Cmyk,
            _ => Space::Gray,
        };
        let width = be32(h, 12)? as usize;
        let height = be32(h, 16)? as usize;
        if width == 0 || height == 0 || width > 20000 || height > 100_000 {
            return None;
        }
        let (img, next) = decode_page(b, i + 32, &Page { width, height, bpp, space, urf: true })?;
        pages.push(img);
        i = next;
    }
    Some(pages)
}

/// Decodes a submitted document according to its MIME type (or sniffing).
pub fn decode(format: &str, b: &[u8]) -> Option<Vec<DynamicImage>> {
    let gray = |v: Vec<GrayImage>| v.into_iter().map(DynamicImage::ImageLuma8).collect::<Vec<_>>();
    match format {
        "image/pwg-raster" => pwg(b).map(gray),
        "image/urf" => urf(b).map(gray),
        "image/jpeg" | "image/png" => image::load_from_memory(b).ok().map(|i| vec![i]),
        _ => pwg(b).map(gray).or_else(|| urf(b).map(gray)).or_else(|| image::load_from_memory(b).ok().map(|i| vec![i])),
    }
}

/// Removes blank margins so a mostly empty A4 page does not waste roll paper.
/// Left/right margins are trimmed only symmetrically to keep layouts centred.
pub fn trim(img: &DynamicImage) -> Option<DynamicImage> {
    let g = img.to_luma8();
    let (w, h) = g.dimensions();
    let ink = |x: u32, y: u32| g.get_pixel(x, y)[0] < 200;
    let rows: Vec<bool> = (0..h).map(|y| (0..w).any(|x| ink(x, y))).collect();
    let top = rows.iter().position(|&r| r)? as u32;
    let bottom = rows.iter().rposition(|&r| r)? as u32;
    let left = (0..w).find(|&x| (top..=bottom).any(|y| ink(x, y))).unwrap_or(0);
    let right = (0..w).rev().find(|&x| (top..=bottom).any(|y| ink(x, y))).unwrap_or(w - 1);
    let side = left.min(w - 1 - right);
    let pad = (w / 60).max(4);
    let x0 = side.saturating_sub(pad);
    let x1 = (w - side + pad).min(w);
    let y0 = top.saturating_sub(pad);
    let y1 = (bottom + pad).min(h);
    Some(img.crop_imm(x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pwg_header(w: u32, h: u32, bpp: u32, cs: u32) -> Vec<u8> {
        let mut hd = vec![0u8; 1796];
        hd[372..376].copy_from_slice(&w.to_be_bytes());
        hd[376..380].copy_from_slice(&h.to_be_bytes());
        hd[384..388].copy_from_slice(&bpp.to_be_bytes());
        hd[388..392].copy_from_slice(&bpp.to_be_bytes());
        hd[400..404].copy_from_slice(&cs.to_be_bytes());
        hd
    }

    #[test]
    fn pwg_gray_page() {
        let mut b = b"RaS2".to_vec();
        b.extend(pwg_header(4, 3, 8, 18));
        // Line 1 repeated twice: 2 black then 2 white; line 3: literal 4 pixels.
        b.extend([1, 1, 0x00, 1, 0xFF]);
        b.extend([0, 0xFD, 10, 20, 30, 40]);
        let p = pwg(&b).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].get_pixel(0, 1)[0], 0);
        assert_eq!(p[0].get_pixel(3, 0)[0], 255);
        assert_eq!(p[0].get_pixel(2, 2)[0], 30);
    }

    #[test]
    fn urf_fill_white() {
        let mut b = b"UNIRAST\0".to_vec();
        b.extend(1u32.to_be_bytes());
        let mut h = vec![0u8; 32];
        h[0] = 8;
        h[1] = 0;
        h[12..16].copy_from_slice(&5u32.to_be_bytes());
        h[16..20].copy_from_slice(&1u32.to_be_bytes());
        b.extend(h);
        b.extend([0, 0x00, 0x00, 0x80]);
        let p = urf(&b).unwrap();
        assert_eq!(p[0].get_pixel(0, 0)[0], 0);
        assert_eq!(p[0].get_pixel(4, 0)[0], 255);
    }
}
