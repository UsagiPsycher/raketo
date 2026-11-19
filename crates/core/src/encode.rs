//! Print job encoders. Each one mirrors an encoder of the app:
//! standard (`g0.a` + `b0`), gray (`c0`), A4 (`d0`) and newFormat (`e0`).

use crate::frame::{self, cmd, Mode};
use crate::raster::{self, Bitmap, Dither, GrayBitmap};
use crate::Model;
use image::GrayImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Content {
    /// Text and line art: threshold, text energy and speed.
    Text,
    /// Graphics: Floyd dithering, image energy.
    Image,
    /// Photos: Floyd dithering, photo mode.
    Photo,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobOptions {
    /// 1 (light) ..= 7 (dark); 4 is the app default.
    pub density: u8,
    pub content: Content,
    /// 16-level gray printing, when the model supports it.
    pub gray: bool,
    /// Label paper (gap detection) instead of continuous roll.
    pub label: bool,
    pub copies: u32,
    /// Firmware version reported by the printer (cmd 0xA8), used for LZO selection.
    pub firmware: Option<String>,
    /// Overrides the computed energy.
    pub energy: Option<u32>,
    /// Overrides the model speed.
    pub speed: Option<u32>,
    /// Extra blank lines fed after the job, on top of the model default.
    pub extra_feed: u32,
    /// Per-printer gray calibration; `None` keeps the app's mapping.
    pub gray_curve: Option<GrayCurve>,
    /// Label height, bounds the gap search on label paper.
    pub label_height_mm: Option<u32>,
}

/// Maps requested darkness 0..=15 into the range a printer actually renders.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GrayCurve {
    /// Highest level that still prints white (first visible step − 1).
    pub lo: u8,
    /// First level that prints full black.
    pub hi: u8,
    /// < 1 darkens mid-tones, > 1 lightens them.
    pub gamma: f32,
}

impl GrayCurve {
    pub fn apply(&self, level: u8) -> u8 {
        if level == 0 {
            return 0;
        }
        let (lo, hi) = (self.lo.min(14) as f32, self.hi.clamp(self.lo + 1, 15) as f32);
        let t = ((level.min(15) - 1) as f32 / 14.0).powf(self.gamma.clamp(0.2, 3.0));
        (lo + 1.0 + t * (hi - lo - 1.0)).round().clamp(0.0, 15.0) as u8
    }
}

impl Default for JobOptions {
    fn default() -> Self {
        JobOptions {
            density: 4,
            content: Content::Image,
            gray: false,
            label: false,
            copies: 1,
            firmware: None,
            energy: None,
            speed: None,
            extra_feed: 0,
            gray_curve: None,
            label_height_mm: None,
        }
    }
}

/// Encoder the app picks for a model and job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Standard,
    Gray,
    A4,
    EscPos,
}

pub fn encoder_for(model: &Model, opts: &JobOptions) -> Encoder {
    if model.new_format {
        Encoder::EscPos
    } else if model.is_a4() {
        Encoder::A4
    } else if opts.gray && model.gray_print {
        Encoder::Gray
    } else {
        Encoder::Standard
    }
}

/// `h0.getPrintEneragy`: base × (1 + 0.15 × (density − 4)).
pub fn energy_for(model: &Model, opts: &JobOptions, gray: bool) -> u32 {
    if let Some(e) = opts.energy {
        return e;
    }
    let base = match (opts.content, gray && model.gray_print) {
        (Content::Text, false) => model.text_eneragy,
        (Content::Text, true) => model.moderation_eneragy,
        (_, true) => model.gray_moderation_eneragy,
        (_, false) => model.moderation_eneragy,
    } as f64;
    let d = opts.density.clamp(1, 7) as f64 - 4.0;
    (d * 0.15 * base + base).max(0.0) as u32
}

fn dither_for(c: Content) -> Dither {
    match c {
        Content::Text => Dither::Threshold,
        _ => Dither::Floyd,
    }
}

/// Packs one row of dots. Bit 0 is the leftmost pixel, except on A4XII heads.
pub fn pack_row(row: &[u8], msb_first: bool) -> Vec<u8> {
    row.chunks(8)
        .map(|c| {
            c.iter().enumerate().fold(0u8, |b, (i, &d)| {
                if d == 0 {
                    b
                } else if msb_first {
                    b | (0x80 >> i)
                } else {
                    b | (1 << i)
                }
            })
        })
        .collect()
}

/// Run-length encoding of a row: each byte is `color << 7 | run` with runs ≤ 127.
pub fn rle_row(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let c = row[i];
        let mut n = 1;
        while i + n < row.len() && row[i + n] == c {
            n += 1;
        }
        let mut left = n;
        while left > 0 {
            let r = left.min(127);
            out.push((c << 7) | r as u8);
            left -= r;
        }
        i += n;
    }
    out
}

fn compress(model: &Model, raw: &[u8]) -> Vec<u8> {
    if model.core_print {
        use flate2::{write::ZlibEncoder, Compression};
        use std::io::Write;
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(raw).ok();
        e.finish().unwrap_or_default()
    } else {
        lzokay_native::compress(raw).unwrap_or_default()
    }
}

fn feed_lines(model: &Model) -> u32 {
    if model.devdpi == 300 {
        72
    } else {
        48
    }
}

fn mode_for(c: Content) -> Mode {
    match c {
        Content::Text => Mode::Text,
        Content::Image => Mode::Image,
        Content::Photo => Mode::Photo,
    }
}

fn speed_for(model: &Model, opts: &JobOptions) -> u32 {
    opts.speed.unwrap_or(match opts.content {
        Content::Text => model.text_print_speed,
        _ => model.img_print_speed,
    })
}

/// Page body of `g0.a`: energy, mode, speed and rows.
fn standard_body(model: &Model, bmp: &Bitmap, opts: &JobOptions) -> Vec<u8> {
    let professional = model.model_no == "Professional Printer";
    let mut out = Vec::new();
    let mut energy = if professional {
        match opts.density {
            4 => model.moderation_eneragy,
            d if d > 4 => model.deepen_eneragy,
            _ => model.thin_eneragy,
        }
    } else {
        energy_for(model, opts, false)
    };
    if opts.label && model.model_no != "U1" && opts.energy.is_none() {
        let img = JobOptions { content: Content::Image, ..opts.clone() };
        energy = energy_for(model, &img, false) * 2;
    }
    if energy != 0 {
        out.extend(frame::energy(energy));
    }
    out.extend(frame::mode(mode_for(opts.content)));
    let speed = speed_for(model, opts);
    let bd = frame::speed(speed);
    out.extend(&bd);

    let msb = model.a4xii;
    if model.uses_lzo(opts.firmware.as_deref()) {
        let lines = if model.devdpi == 300 { 10 } else { 18 };
        let packed: Vec<u8> = (0..bmp.height).flat_map(|y| pack_row(bmp.row(y), msb)).collect();
        let stride = (bmp.width as usize).div_ceil(8) * lines;
        for chunk in packed.chunks(stride) {
            out.extend(frame::block(cmd::LZO_BLOCK, chunk.len(), &compress(model, chunk)));
            if !professional {
                out.extend(&bd);
            }
        }
        return out;
    }
    let row_bytes = (bmp.width as usize).div_ceil(8);
    for y in 0..bmp.height {
        let row = bmp.row(y);
        let rle = if model.new_compress { Some(rle_row(row)) } else { None };
        match rle {
            Some(r) if r.len() <= row_bytes => out.extend(frame::frame(cmd::ROW_RLE, &r)),
            _ => out.extend(frame::frame(cmd::ROW_RAW, &pack_row(row, msb))),
        }
        if !professional && (y + 1) % 200 == 0 {
            out.extend(&bd);
        }
    }
    out
}

fn job_standard(model: &Model, pages: &[Bitmap], opts: &JobOptions) -> Vec<u8> {
    let label = opts.label && model.can_print_label;
    let label_printer = model.model_no == "Label Printer";
    let mut out = frame::quality(model, opts.density);
    if label_printer {
        out.extend(if label { frame::check_label_location() } else { frame::feed(4) });
    } else if label && model.label_up_paper_num != 0 {
        out.extend(frame::feed_check_black(model.label_up_paper_num));
    }
    for _ in 0..opts.copies.max(1) {
        for p in pages {
            out.extend(standard_body(model, p, opts));
        }
    }
    let fl = feed_lines(model);
    if model.auto_label_check || !label {
        out.extend(frame::speed(25));
        if !label {
            for _ in 0..model.paper_num.max(2) {
                out.extend(frame::feed(fl));
            }
            if opts.extra_feed > 0 {
                out.extend(frame::feed(opts.extra_feed));
            }
        }
    } else {
        // Let the firmware run up to the next gap or black mark.
        out.extend(frame::feed_check_black(500));
    }
    if label {
        for _ in 0..model.paper_num.saturating_sub(2) {
            out.extend(frame::feed(fl));
        }
    } else if !model.auto_label_check {
        out.extend(frame::dev_info());
    }
    out.extend(frame::status());
    out
}

/// Gap search step of the app (`LongDetectionLabel`): feed one line and read
/// the paper sensor, `n` times.
pub fn label_probe(n: u32) -> Vec<u8> {
    (0..n).flat_map(|_| [frame::feed(1), frame::status()].concat()).collect()
}

/// Moves from the detected gap to the printing position (`upLabel`).
pub fn label_advance(model: &Model) -> Vec<u8> {
    let mut v = frame::feed(model.back_paper_num.max(0) as u32);
    v.extend(frame::status());
    v
}

/// Lets models with hardware gap detection find the next label themselves.
pub fn label_auto_check() -> Vec<u8> {
    frame::feed_check_black(500)
}

/// Port of the app's gap detection on the paper sensor readings.
///
/// `detectionValue` 1: a gap reads above the paper baseline, the label edge is
/// a fall back to it; 2: the opposite. The baseline is learnt from stable readings.
#[derive(Debug, Clone)]
pub struct LabelDetector {
    pub baseline: i32,
    mode: i32,
    old: i32,
    stable_val: i32,
    stable_count: u32,
    /// Set when a new baseline has been learnt.
    pub learnt: Option<i32>,
}

impl LabelDetector {
    pub fn new(model: &Model, baseline: Option<i32>) -> Self {
        let def = baseline.unwrap_or_else(|| model.label_value.parse().unwrap_or(1));
        LabelDetector { baseline: def, mode: model.detection_value, old: 0, stable_val: 0, stable_count: 0, learnt: None }
    }

    /// Feeds one sensor reading; returns true on a label edge.
    pub fn push(&mut self, r: u8) -> bool {
        let r = r as i32;
        if self.stable_val == 0 {
            self.stable_val = r;
        } else if (self.stable_val - r).abs() <= 1 {
            self.stable_count += 1;
            if self.stable_count == 100 {
                self.stable_count = 0;
                self.baseline = self.stable_val;
                self.learnt = Some(self.baseline);
            }
        } else {
            self.stable_val = r;
            self.stable_count = 0;
        }
        let mut edge = false;
        if self.old != 0 {
            edge = match self.mode {
                1 => r <= self.baseline + 1 && self.old > self.baseline + 1,
                2 => r >= self.baseline - 1 && self.old < self.baseline - 1,
                _ => false,
            };
        }
        self.old = r;
        edge
    }
}

/// 4-bit gray data as sent by `convert4ColorImageToPointData`.
fn gray_data(model: &Model, g: &GrayBitmap, bd: &[u8]) -> Vec<u8> {
    let half = (g.width as usize).div_ceil(2);
    let mut buf = vec![0u8; half * 16];
    for y in 0..g.height as usize {
        let row = &g.levels[y * g.width as usize..(y + 1) * g.width as usize];
        for pair in row.chunks(2) {
            let lo = pair[0] & 0x0F;
            let hi = pair.get(1).copied().unwrap_or(0) & 0x0F;
            buf.push((hi << 4) | lo);
        }
    }
    if buf.len() % 4 != 0 {
        let pad = buf.len() % 4;
        buf.extend(std::iter::repeat_n(0, pad));
    }
    let raw = model.model_no == "FL01" || model.model_no == "YMS-BT01";
    let rows = if model.model_no == "DL_X7" || model.model_no == "YMS-BT01" { 1 } else { 20 };
    let mut out = Vec::new();
    for chunk in buf.chunks(half * rows) {
        let payload = if raw { chunk.to_vec() } else { lzokay_native::compress(chunk).unwrap_or_default() };
        out.extend(frame::block(cmd::GRAY_BLOCK, chunk.len(), &payload));
        out.extend(bd);
    }
    out
}

fn job_gray(model: &Model, pages: &[GrayBitmap], opts: &JobOptions) -> Vec<u8> {
    let mut out = frame::quality(model, opts.density);
    for _ in 0..opts.copies.max(1) {
        for g in pages {
            let e = energy_for(model, opts, true);
            if e != 0 {
                out.extend(frame::energy(e));
            }
            out.extend(frame::frame(cmd::MODE, &[0x00, 0x01]));
            let speed = opts.speed.unwrap_or(model.gray_image_speed);
            let bd = frame::speed(speed);
            out.extend(&bd);
            out.extend(gray_data(model, g, &bd));
        }
    }
    out.extend(frame::speed(15));
    if !opts.label {
        for _ in 0..model.paper_num.max(2) {
            out.extend(frame::feed(feed_lines(model)));
        }
        if opts.extra_feed > 0 {
            out.extend(frame::feed(opts.extra_feed));
        }
    }
    out.extend(frame::status());
    out
}

fn job_a4(model: &Model, pages: &[Bitmap], opts: &JobOptions) -> Vec<u8> {
    let mut out = Vec::new();
    let pad = model.left_pixels();
    for _ in 0..opts.copies.max(1) {
        for p in pages {
            if model.model_no == "Professional Printer" {
                out.extend(frame::stop());
            }
            out.extend(frame::quality(model, opts.density));
            out.extend(standard_body(model, &p.pad_left(pad), opts));
            if model.a4xii {
                out.extend(frame::feed_check_black(100));
            } else {
                let n = model.paper_num as i32 * feed_lines(model) as i32;
                out.extend(frame::feed_check_black(n));
            }
            if !model.core_print {
                out.extend(frame::status());
            }
        }
    }
    out
}

/// ESC/POS column graphics used by newFormat models (`e0`).
fn job_escpos(model: &Model, pages: &[Bitmap], opts: &JobOptions) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..opts.copies.max(1) {
        for p in pages {
            let w = p.width as usize;
            let bands = (p.height as usize).div_ceil(24);
            let mut dots = p.dots.clone();
            dots.resize(bands * 24 * w, 0);
            out.extend([0x1B, 0x40, 0x12, 0x23]);
            let e = if opts.content == Content::Text {
                let t = model.text_eneragy as i32;
                match opts.density {
                    0..=3 => t - 2,
                    4 => t,
                    _ => t + 2,
                }
            } else {
                opts.energy.unwrap_or(model.moderation_eneragy) as i32
            };
            out.push(e.clamp(0, 255) as u8);
            let m = if opts.content == Content::Text { 1 } else { 0 };
            out.extend(frame::frame_prefixed(cmd::MODE, &[m]));
            for b in 0..bands {
                out.extend([0x1B, 0x2A, 0x21, (w & 0xFF) as u8, (w >> 8) as u8]);
                for x in 0..w {
                    for k in 0..3 {
                        let mut byte = 0u8;
                        for bit in 0..8 {
                            let y = b * 24 + k * 8 + bit;
                            if dots[y * w + x] == 1 {
                                byte |= 0x80 >> bit;
                            }
                        }
                        out.push(byte);
                    }
                }
                out.extend([0x1B, 0x33, 0x00, 0x0A]);
            }
        }
    }
    out.extend([0x1B, 0x64, if model.devdpi == 300 { 4 } else { 3 }]);
    out.extend(frame::frame_prefixed(cmd::STATUS, &[0]));
    out
}

/// A page prepared for a model: what will be sent, plus a preview.
pub enum Raster {
    Mono(Bitmap),
    Gray(GrayBitmap),
}

impl Raster {
    pub fn preview(&self) -> GrayImage {
        match self {
            Raster::Mono(b) => b.to_image(),
            Raster::Gray(g) => g.to_image(),
        }
    }
}

/// Converts pages (already laid out at any width) to the model raster.
pub fn rasterize(model: &Model, pages: &[image::DynamicImage], opts: &JobOptions) -> Vec<Raster> {
    let width = model.image_width();
    let gray = encoder_for(model, opts) == Encoder::Gray;
    pages
        .iter()
        .map(|p| {
            let g = raster::fit_width(p, width);
            if gray {
                Raster::Gray(raster::to_gray(&g, model.gray_scale))
            } else {
                Raster::Mono(raster::to_bitmap(&g, dither_for(opts.content)))
            }
        })
        .collect()
}

/// Builds the complete byte stream for a job.
pub fn build_job(model: &Model, pages: &[Raster], opts: &JobOptions) -> Vec<u8> {
    let mono: Vec<Bitmap> = pages
        .iter()
        .filter_map(|r| if let Raster::Mono(b) = r { Some(b.clone()) } else { None })
        .collect();
    match encoder_for(model, opts) {
        Encoder::Gray => {
            let gray: Vec<GrayBitmap> = pages
                .iter()
                .filter_map(|r| if let Raster::Gray(g) = r { Some(g.clone()) } else { None })
                .map(|mut g| {
                    if let Some(c) = opts.gray_curve {
                        g.levels.iter_mut().for_each(|l| *l = c.apply(*l));
                    }
                    g
                })
                .collect();
            job_gray(model, &gray, opts)
        }
        Encoder::Standard => job_standard(model, &mono, opts),
        Encoder::A4 => job_a4(model, &mono, opts),
        Encoder::EscPos => job_escpos(model, &mono, opts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_splits_long_runs() {
        let mut row = vec![0u8; 384];
        row[10] = 1;
        let r = rle_row(&row);
        assert_eq!(r, vec![10, 0x81, 127, 127, 119]);
    }

    #[test]
    fn pack_is_lsb_first() {
        assert_eq!(pack_row(&[1, 0, 0, 0, 0, 0, 0, 1], false), vec![0x81]);
        assert_eq!(pack_row(&[1, 1, 0, 0, 0, 0, 0, 0], false), vec![0x03]);
        assert_eq!(pack_row(&[1, 1, 0, 0, 0, 0, 0, 0], true), vec![0xC0]);
    }

    #[test]
    fn gray_curve_spans_visible_range() {
        let c = GrayCurve { lo: 4, hi: 13, gamma: 1.0 };
        assert_eq!(c.apply(0), 0);
        assert_eq!(c.apply(1), 5);
        assert_eq!(c.apply(15), 13);
    }

    #[test]
    fn label_edge_detection() {
        let m = crate::lookup("SC03h-1").unwrap();
        let mut d = LabelDetector::new(m, None);
        // Paper reads ~1, the gap reads high, then paper again.
        let seq = [1u8, 1, 2, 9, 12, 12, 8, 2, 1];
        let edges: Vec<bool> = seq.iter().map(|&r| d.push(r)).collect();
        assert_eq!(edges.iter().filter(|&&e| e).count(), 1);
        assert!(edges[7]);
    }

    #[test]
    fn energy_formula() {
        let m = crate::lookup("SC03h-1").unwrap();
        let o = JobOptions::default();
        assert_eq!(energy_for(m, &o, false), 5000);
        let dark = JobOptions { density: 7, ..o };
        assert_eq!(energy_for(m, &dark, false), 7250);
    }

    #[test]
    fn every_model_encodes() {
        let img = image::DynamicImage::ImageLuma8(GrayImage::from_fn(200, 50, |x, _| image::Luma([(x % 256) as u8])));
        for m in crate::models() {
            for gray in [false, true] {
                let o = JobOptions { gray, ..Default::default() };
                let r = rasterize(m, std::slice::from_ref(&img), &o);
                let job = build_job(m, &r, &o);
                assert!(job.len() > 50, "{}", m.model_no);
            }
        }
    }
}
