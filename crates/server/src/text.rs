//! Plain text → page image, for `POST /print/text` and the CLI.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont, VariableFont};
use image::{GrayImage, Luma};

static FONT: &[u8] = include_bytes!("../fonts/BricolageGrotesque.ttf");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Style {
    pub size: f32,
    pub bold: bool,
    pub align: Align,
}

fn font(bold: bool, size: f32) -> FontRef<'static> {
    let mut f = FontRef::try_from_slice(FONT).expect("embedded font");
    f.set_variation(b"wght", if bold { 700.0 } else { 420.0 });
    f.set_variation(b"opsz", size.clamp(12.0, 96.0));
    f
}

fn width_of<F: Font>(f: &ab_glyph::PxScaleFont<&F>, s: &str) -> f32 {
    let mut w = 0.0;
    let mut prev = None;
    for c in s.chars() {
        let g = f.glyph_id(c);
        if let Some(p) = prev {
            w += f.kern(p, g);
        }
        w += f.h_advance(g);
        prev = Some(g);
    }
    w
}

fn wrap<F: Font>(f: &ab_glyph::PxScaleFont<&F>, text: &str, max: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.replace("\r\n", "\n").split('\n') {
        let mut line = String::new();
        for word in para.split_inclusive(' ') {
            let candidate = format!("{line}{word}");
            if width_of(f, candidate.trim_end()) <= max || line.is_empty() {
                if width_of(f, candidate.trim_end()) > max {
                    // A word wider than the line is split by characters.
                    let mut chunk = String::new();
                    for c in word.chars() {
                        if width_of(f, &format!("{chunk}{c}")) > max && !chunk.is_empty() {
                            out.push(chunk.clone());
                            chunk.clear();
                        }
                        chunk.push(c);
                    }
                    line = chunk;
                } else {
                    line = candidate;
                }
            } else {
                out.push(line.trim_end().to_string());
                line = word.to_string();
            }
        }
        out.push(line.trim_end().to_string());
    }
    // Drop trailing empty lines so the paper stops at the text.
    while out.len() > 1 && out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Renders `text` black on white, `width` pixels wide.
pub fn render(text: &str, width: u32, style: &Style) -> GrayImage {
    let size = style.size.clamp(8.0, 400.0);
    let f = font(style.bold, size);
    let sf = f.as_scaled(PxScale::from(size));
    let pad = 4.0;
    let lines = wrap(&sf, text, width as f32 - pad * 2.0);
    let line_h = (size * 1.25).ceil();
    let height = (lines.len() as f32 * line_h + size * 0.3).ceil() as u32;
    let mut img = GrayImage::from_pixel(width, height.max(1), Luma([255]));
    for (i, line) in lines.iter().enumerate() {
        let w = width_of(&sf, line);
        let x0 = match style.align {
            Align::Left => pad,
            Align::Center => (width as f32 - w) / 2.0,
            Align::Right => width as f32 - pad - w,
        };
        let baseline = i as f32 * line_h + sf.ascent() + (line_h - (sf.ascent() - sf.descent())) / 2.0;
        let mut x = x0;
        let mut prev = None;
        for c in line.chars() {
            let gid = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, gid);
            }
            let glyph = gid.with_scale_and_position(PxScale::from(size), point(x, baseline));
            if let Some(o) = f.outline_glyph(glyph) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let px = b.min.x as i32 + gx as i32;
                    let py = b.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < img.height() {
                        let p = img.get_pixel_mut(px as u32, py as u32);
                        let v = 255.0 * (1.0 - cov.clamp(0.0, 1.0));
                        p[0] = p[0].min(v as u8);
                    }
                });
            }
            x += sf.h_advance(gid);
            prev = Some(gid);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_draws() {
        let img = render("Hello Raketo, this line is long enough to wrap\nsecond", 384, &Style { size: 32.0, bold: true, align: Align::Left });
        assert_eq!(img.width(), 384);
        assert!(img.height() > 32 * 3);
        assert!(img.pixels().any(|p| p[0] < 50));
    }
}
