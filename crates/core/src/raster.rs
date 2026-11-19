//! Image → printer raster, following the app's preprocessing.

use image::{imageops::FilterType, DynamicImage, GrayImage, Luma};

/// 1 bit per pixel, stored one byte per pixel (1 = black) for easy packing.
#[derive(Debug, Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub dots: Vec<u8>,
}

/// 4 bits per pixel darkness (0 = white, 15 = black), one byte per pixel.
#[derive(Debug, Clone)]
pub struct GrayBitmap {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dither {
    /// Mean-based threshold, best for text and line art.
    Threshold,
    /// Floyd–Steinberg after the app's tone curve, best for photos.
    Floyd,
}

/// Flattens transparency on white and scales to `width`, keeping the aspect ratio.
pub fn fit_width(img: &DynamicImage, width: u32) -> GrayImage {
    let rgba = img.to_rgba8();
    let mut flat = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        flat.put_pixel(x, y, image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])]));
    }
    let flat = DynamicImage::ImageRgb8(flat);
    let h = ((flat.height() as u64 * width as u64) / flat.width().max(1) as u64).max(1) as u32;
    let scaled = if flat.width() == width {
        flat
    } else {
        flat.resize_exact(width, h, FilterType::Triangle)
    };
    scaled.to_luma8()
}

fn sharpen(g: &GrayImage, s: f32) -> Vec<f32> {
    let (w, h) = (g.width() as i32, g.height() as i32);
    let at = |x: i32, y: i32| g.get_pixel(x.clamp(0, w - 1) as u32, y.clamp(0, h - 1) as u32)[0] as f32;
    let mut out = vec![0f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let v = at(x, y) * (1.0 + 4.0 * s)
                - s * (at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1));
            out[(y * w + x) as usize] = v.clamp(0.0, 255.0);
        }
    }
    out
}

/// Auto-levels + tone curve from `convertGreyImgByFloydPixels`.
fn tone_curve(px: &mut [f32], low_cap: i32, high_floor: i32, value_scale: f64) {
    let (high_t, low_t, scale) = (0.2f32, 0.2f32, 0.46f32);
    let mut hist = [0u32; 256];
    for &v in px.iter() {
        hist[v as usize] += 1;
    }
    let n = px.len() as f32;
    let (lo_n, hi_n) = ((n * low_t) as u64, (n * high_t) as u64);
    let mut acc = 0u64;
    let mut low = 0i32;
    for (i, &c) in hist.iter().enumerate() {
        acc += c as u64;
        if acc > lo_n {
            low = i as i32;
            break;
        }
    }
    acc = 0;
    let mut high = 0i32;
    for i in (1..256).rev() {
        acc += hist[i] as u64;
        if acc > hi_n {
            high = i as i32;
            break;
        }
    }
    low = low.min(low_cap);
    high = high.max(high_floor);
    let top = |v: f32| (1.0 - scale) * (255.0 - v) + v;
    for v in px.iter_mut() {
        let i = *v as i32;
        let out = if i <= low {
            i as f32 * scale
        } else if i >= high {
            top(i as f32)
        } else {
            let f16 = low as f32 * scale;
            (top(high as f32) - f16) * (i - low) as f32 / (high - low).max(1) as f32 + f16
        };
        *v = ((out as i32) as f64 * value_scale).clamp(0.0, 255.0) as f32;
    }
}

fn diffuse(px: &mut [f32], w: usize, h: usize, quant: impl Fn(f32) -> f32) {
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let old = px[i];
            let new = quant(old);
            px[i] = new;
            let e = old - new;
            if x + 1 < w {
                px[i + 1] += e * 7.0 / 16.0;
            }
            if y + 1 < h {
                if x > 0 {
                    px[i + w - 1] += e * 3.0 / 16.0;
                }
                px[i + w] += e * 5.0 / 16.0;
                if x + 1 < w {
                    px[i + w + 1] += e / 16.0;
                }
            }
        }
    }
}

pub fn to_bitmap(g: &GrayImage, dither: Dither) -> Bitmap {
    let (w, h) = (g.width(), g.height());
    let dots = match dither {
        Dither::Threshold => {
            let sum: u64 = g.pixels().map(|p| p[0] as u64).sum();
            let thr = (sum / (w as u64 * h as u64).max(1)) as i32 - 13;
            g.pixels()
                .map(|p| {
                    let v = p[0] as i32;
                    (v == 0 || v <= thr) as u8
                })
                .collect()
        }
        Dither::Floyd => {
            let mut px = sharpen(g, 1.1);
            tone_curve(&mut px, 70, 185, 1.0);
            diffuse(&mut px, w as usize, h as usize, |v| if v < 128.0 { 0.0 } else { 255.0 });
            px.iter().map(|&v| (v < 128.0) as u8).collect()
        }
    };
    Bitmap { width: w, height: h, dots }
}

pub fn to_gray(g: &GrayImage, gray_scale: f64) -> GrayBitmap {
    let (w, h) = (g.width(), g.height());
    let mut px = sharpen(g, 1.1);
    let scale = if gray_scale > 0.0 { gray_scale } else { 1.0 };
    tone_curve(&mut px, 110, 150, scale);
    diffuse(&mut px, w as usize, h as usize, |v| ((v as i32 / 18) * 19) as f32);
    let levels = px
        .iter()
        .map(|&v| {
            let mut l = (v / (256.0 / 14.0)) as i32;
            if l == 13 {
                l = 15;
            }
            15 - l.clamp(0, 15) as u8
        })
        .collect();
    GrayBitmap { width: w, height: h, levels }
}

impl Bitmap {
    pub fn blank(width: u32, height: u32) -> Self {
        Bitmap { width, height, dots: vec![0; (width * height) as usize] }
    }

    /// Adds white columns on the left (A4 heads).
    pub fn pad_left(&self, n: u32) -> Bitmap {
        if n == 0 {
            return self.clone();
        }
        let w = self.width + n;
        let mut dots = vec![0u8; (w * self.height) as usize];
        for y in 0..self.height as usize {
            let src = &self.dots[y * self.width as usize..(y + 1) * self.width as usize];
            dots[y * w as usize + n as usize..(y + 1) * w as usize].copy_from_slice(src);
        }
        Bitmap { width: w, height: self.height, dots }
    }

    pub fn row(&self, y: u32) -> &[u8] {
        let w = self.width as usize;
        &self.dots[y as usize * w..(y as usize + 1) * w]
    }

    /// Preview as a grayscale image (black dots on white).
    pub fn to_image(&self) -> GrayImage {
        GrayImage::from_fn(self.width, self.height, |x, y| {
            Luma([if self.dots[(y * self.width + x) as usize] == 1 { 0 } else { 255 }])
        })
    }
}

impl GrayBitmap {
    pub fn to_image(&self) -> GrayImage {
        GrayImage::from_fn(self.width, self.height, |x, y| {
            Luma([255 - self.levels[(y * self.width + x) as usize] * 17])
        })
    }
}
