//! PDF and EPUB documents → page images at the printer width.

use crate::text;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, Luma};
use std::io::{Cursor, Read};

pub fn is_pdf(b: &[u8]) -> bool {
    b.starts_with(b"%PDF")
}

pub fn is_epub(b: &[u8]) -> bool {
    b.starts_with(b"PK\x03\x04")
        && (b.get(30..58) == Some(b"mimetypeapplication/epub+zip".as_slice())
            || zip::ZipArchive::new(Cursor::new(b)).is_ok_and(|mut z| z.by_name("META-INF/container.xml").is_ok()))
}

/// Parses "1-3, 5, 8-" into 0-based indexes below `count`; empty means all.
pub fn parse_range(s: Option<&str>, count: usize) -> Vec<usize> {
    let s = s.unwrap_or("").trim();
    if s.is_empty() {
        return (0..count).collect();
    }
    let mut out = Vec::new();
    for part in s.split([',', ';', ' ']).filter(|p| !p.is_empty()) {
        let (a, b): (Option<usize>, Option<usize>) = match part.split_once('-') {
            Some((a, "")) => (a.parse().ok(), Some(count)),
            Some((a, b)) => (a.parse().ok(), b.parse().ok()),
            None => (part.parse().ok(), part.parse().ok()),
        };
        if let (Some(a), Some(b)) = (a, b) {
            for i in a.max(1)..=b.min(count) {
                if !out.contains(&(i - 1)) {
                    out.push(i - 1);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// Renders the selected pages of a PDF, `width` pixels wide.
pub fn render_pdf(data: &[u8], width: u32, range: Option<&str>) -> Result<Vec<GrayImage>, String> {
    let data = data.to_vec();
    let range = range.map(str::to_string);
    // The renderer is young; a broken file must not take the server down.
    std::panic::catch_unwind(move || {
        let pdf = Pdf::new(data).map_err(|_| "the PDF could not be read".to_string())?;
        let pages = pdf.pages();
        let cache = RenderCache::new();
        let settings = InterpreterSettings::default();
        let mut out = Vec::new();
        for i in parse_range(range.as_deref(), pages.len()) {
            let page = &pages[i];
            let (w, _) = page.render_dimensions();
            let s = width as f32 / w.max(1.0);
            let pm = hayro::render(page, &cache, &settings, &RenderSettings::default(), &PixmapSettings { x_scale: s, y_scale: s, bg_color: WHITE });
            let (pw, ph) = (pm.width() as u32, pm.height() as u32);
            let rgba = image::RgbaImage::from_raw(pw, ph, pm.data_as_u8_slice().to_vec()).ok_or("bad page size")?;
            out.push(DynamicImage::ImageRgba8(rgba).to_luma8());
        }
        if out.is_empty() {
            return Err("no page in that range".into());
        }
        Ok(out)
    })
    .unwrap_or_else(|_| Err("the PDF could not be rendered".into()))
}

// ---------- EPUB ----------

#[derive(Debug, PartialEq)]
enum Block {
    Heading(u8, String),
    Text(String),
    Pre(String),
    Image(String),
    Rule,
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "copy" => Some('©'),
            "middot" => Some('·'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before = lower[..at].chars().last();
        if !matches!(before, Some(c) if c.is_whitespace() || c == ':') {
            continue;
        }
        let after = tag[from..].trim_start();
        let Some(after) = after.strip_prefix('=') else { continue };
        let after = after.trim_start();
        let (q, body) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &after[1..]),
            _ => (None, after),
        };
        let end = match q {
            Some(q) => body.find(q),
            None => body.find(|c: char| c.is_whitespace() || c == '>' || c == '/'),
        }
        .unwrap_or(body.len());
        return Some(decode_entities(&body[..end]));
    }
    None
}

/// A tolerant walk over (X)HTML: block structure only, inline styling dropped.
fn blocks(html: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut heading: Option<u8> = None;
    let mut pre = 0usize;
    let mut skip: Option<String> = None;
    let flush = |buf: &mut String, out: &mut Vec<Block>, heading: Option<u8>, pre: bool| {
        if pre {
            let t = decode_entities(buf.trim_matches('\n'));
            if !t.trim().is_empty() {
                out.push(Block::Pre(t));
            }
        } else {
            let t: String = decode_entities(&buf.split_whitespace().collect::<Vec<_>>().join(" "));
            let t = t.replace('\u{a0}', " ").trim().to_string();
            if !t.is_empty() {
                out.push(match heading {
                    Some(l) => Block::Heading(l, t),
                    None => Block::Text(t),
                });
            }
        }
        buf.clear();
    };
    let body_start = html.to_ascii_lowercase().find("<body").unwrap_or(0);
    let mut rest = &html[body_start..];
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip.is_none() {
                buf.push_str(rest);
            }
            break;
        };
        if skip.is_none() {
            buf.push_str(&rest[..lt]);
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map(|e| &rest[e + 3..]).unwrap_or("");
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == ':')
            .collect::<String>()
            .to_ascii_lowercase();
        let name = name.rsplit(':').next().unwrap_or("").to_string();
        if let Some(s) = &skip {
            if closing && *s == name {
                skip = None;
            } else if s == "svg" && name == "image" {
                // Cover pages wrap the cover picture in an SVG.
                if let Some(src) = attr(tag, "href") {
                    out.push(Block::Image(src));
                }
            }
            continue;
        }
        match name.as_str() {
            "script" | "style" | "head" | "svg" | "math" | "title" if !closing && !tag.ends_with('/') => skip = Some(name),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush(&mut buf, &mut out, heading, pre > 0);
                heading = if closing { None } else { name[1..].parse().ok() };
            }
            "pre" => {
                flush(&mut buf, &mut out, heading, pre > 0);
                pre = if closing { pre.saturating_sub(1) } else { pre + 1 };
            }
            "br" => {
                if pre > 0 {
                    buf.push('\n');
                } else {
                    flush(&mut buf, &mut out, heading, false);
                }
            }
            "hr" => {
                flush(&mut buf, &mut out, heading, pre > 0);
                out.push(Block::Rule);
            }
            "img" | "image" => {
                flush(&mut buf, &mut out, heading, pre > 0);
                if let Some(src) = attr(tag, "src").or_else(|| attr(tag, "href")) {
                    out.push(Block::Image(src));
                }
            }
            "p" | "div" | "li" | "blockquote" | "tr" | "dt" | "dd" | "section" | "article" | "figcaption" | "table" | "ul" | "ol" | "body" | "td" | "th" => {
                if matches!(name.as_str(), "td" | "th") && !closing {
                    buf.push(' ');
                } else {
                    flush(&mut buf, &mut out, heading, pre > 0);
                    if name == "li" && !closing {
                        buf.push_str("• ");
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut buf, &mut out, heading, pre > 0);
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Joins a relative href to the directory of `base`, inside the archive.
fn resolve(base: &str, href: &str) -> String {
    let href = percent_decode(href.split(['#', '?']).next().unwrap_or(""));
    let mut parts: Vec<&str> = base.rsplit_once('/').map(|(d, _)| d.split('/').collect()).unwrap_or_default();
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

struct Epub {
    zip: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Epub {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut f = self.zip.by_name(path).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).ok()?;
        Some(v)
    }

    fn text(&mut self, path: &str) -> Option<String> {
        self.read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Spine documents in reading order.
    fn spine(&mut self) -> Result<Vec<String>, String> {
        let container = self.text("META-INF/container.xml").ok_or("not an EPUB: META-INF/container.xml is missing")?;
        let opf_path = container
            .split("<rootfile")
            .find_map(|t| attr(t, "full-path"))
            .ok_or("not an EPUB: no package file")?;
        let opf = self.text(&opf_path).ok_or("the EPUB package file is missing")?;
        let mut manifest = std::collections::HashMap::new();
        let is = |t: &str, name: &str| t.split_whitespace().next().is_some_and(|n| n == name || n.ends_with(&format!(":{name}")));
        for item in opf.split('<').filter(|t| is(t, "item")) {
            if let (Some(id), Some(href)) = (attr(item, "id"), attr(item, "href")) {
                manifest.insert(id, resolve(&opf_path, &href));
            }
        }
        let spine: Vec<String> = opf
            .split('<')
            .filter(|t| is(t, "itemref"))
            .filter(|t| attr(t, "linear").as_deref() != Some("no"))
            .filter_map(|t| attr(t, "idref"))
            .filter_map(|id| manifest.get(&id).cloned())
            .collect();
        if spine.is_empty() {
            return Err("the EPUB has no readable content".into());
        }
        Ok(spine)
    }
}

/// Renders the selected chapters of an EPUB as reflowed text, one image each.
pub fn render_epub(data: &[u8], width: u32, range: Option<&str>, size: f32) -> Result<Vec<GrayImage>, String> {
    let zip = zip::ZipArchive::new(Cursor::new(data.to_vec())).map_err(|_| "the EPUB could not be opened")?;
    let mut book = Epub { zip };
    let spine = book.spine()?;
    // Chapters are the spine documents that hold something to print.
    let mut chapters = Vec::new();
    for path in spine {
        let html = book.text(&path).unwrap_or_default();
        let b = blocks(&html);
        if !b.is_empty() {
            chapters.push((path, b));
        }
    }
    let picked = parse_range(range, chapters.len());
    if picked.is_empty() {
        return Err("no chapter in that range".into());
    }
    let size = size.clamp(12.0, 80.0);
    let mut out = Vec::new();
    for i in picked {
        let (path, blocks) = &chapters[i];
        let mut parts: Vec<GrayImage> = Vec::new();
        for b in blocks {
            let style = |size: f32, bold| text::Style { size, bold, align: text::Align::Left };
            match b {
                Block::Heading(l, t) => {
                    let s = size * [1.7, 1.45, 1.25, 1.1, 1.0, 1.0][(*l as usize).clamp(1, 6) - 1];
                    parts.push(GrayImage::from_pixel(width, (size * 0.5) as u32, Luma([255])));
                    parts.push(text::render(t, width, &style(s, true)));
                }
                Block::Text(t) => parts.push(text::render(t, width, &style(size, false))),
                Block::Pre(t) => parts.push(text::render(t, width, &style(size * 0.85, false))),
                Block::Rule => {
                    let mut r = GrayImage::from_pixel(width, (size * 0.8) as u32, Luma([255]));
                    let y = r.height() / 2;
                    for x in width / 4..width * 3 / 4 {
                        r.put_pixel(x, y, Luma([0]));
                        r.put_pixel(x, y + 1, Luma([0]));
                    }
                    parts.push(r);
                }
                Block::Image(src) => {
                    let Some(bytes) = book.read(&resolve(path, src)) else { continue };
                    let Ok(img) = image::load_from_memory(&bytes) else { continue };
                    if img.width() < 24 || img.height() < 24 {
                        continue; // icons and spacers
                    }
                    let w = (img.width() * 2).min(width);
                    let h = (img.height() as u64 * w as u64 / img.width() as u64).max(1) as u32;
                    let small = flatten(&img).resize_exact(w, h, FilterType::Lanczos3).to_luma8();
                    let mut row = GrayImage::from_pixel(width, h + 8, Luma([255]));
                    image::imageops::overlay(&mut row, &small, ((width - w) / 2) as i64, 4);
                    parts.push(row);
                }
            }
        }
        let h: u32 = parts.iter().map(|p| p.height()).sum();
        let mut page = GrayImage::from_pixel(width, h.max(1), Luma([255]));
        let mut y = 0;
        for p in parts {
            image::imageops::overlay(&mut page, &p, 0, y as i64);
            y += p.height();
        }
        out.push(page);
    }
    Ok(out)
}

/// Puts transparent images on white.
fn flatten(img: &DynamicImage) -> DynamicImage {
    let mut rgba = img.to_rgba8();
    for p in rgba.pixels_mut() {
        let a = p[3] as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
        p[3] = 255;
    }
    DynamicImage::ImageRgba8(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 3), vec![0, 1, 2]);
        assert_eq!(parse_range(Some("2-"), 4), vec![1, 2, 3]);
        assert_eq!(parse_range(Some("3, 1"), 4), vec![0, 2]);
        assert_eq!(parse_range(Some("9"), 4), Vec::<usize>::new());
    }

    #[test]
    fn html_blocks() {
        let b = blocks("<html><head><title>x</title></head><body><h1>Title</h1><p>One &amp; <i>two</i>\n  three</p><script>no()</script><img src=\"a%20b.png\"/><ul><li>item</li></ul></body></html>");
        assert_eq!(
            b,
            vec![
                Block::Heading(1, "Title".into()),
                Block::Text("One & two three".into()),
                Block::Image("a%20b.png".into()),
                Block::Text("• item".into()),
            ]
        );
    }

    #[test]
    fn paths() {
        assert_eq!(resolve("OEBPS/text/ch1.xhtml", "../img/a%20b.png"), "OEBPS/img/a b.png");
        assert_eq!(resolve("content.opf", "ch1.html#top"), "ch1.html");
    }
}
