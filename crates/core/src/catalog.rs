//! Printer model catalogue (extracted from the iPrint Android app).
//!
//! The app identifies a printer by its Bluetooth name: the model is the
//! catalogue entry whose `modelNo` is the longest prefix of that name.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Model {
    pub model_no: String,
    /// 1 = Wi‑Fi capable model.
    pub model: i32,
    /// Paper size class; 8 = A4 head.
    pub size: i32,
    pub paper_size: u32,
    pub print_size: u32,
    pub one_length: u32,
    pub head_name: String,
    #[serde(rename = "canChangeMTU")]
    pub can_change_mtu: bool,
    pub devdpi: u32,
    pub img_print_speed: u32,
    pub text_print_speed: u32,
    #[serde(rename = "imgMTU")]
    pub img_mtu: u32,
    /// Allows run-length encoded rows (cmd 0xBF).
    pub new_compress: bool,
    pub paper_num: u32,
    /// Delay between packets, in ms.
    pub interval: u32,
    pub thin_eneragy: u32,
    pub moderation_eneragy: u32,
    pub deepen_eneragy: u32,
    pub text_eneragy: u32,
    pub has_id: bool,
    /// Data goes over classic Bluetooth RFCOMM instead of BLE GATT.
    #[serde(rename = "useSPP")]
    pub use_spp: bool,
    /// ESC/POS-like variant with 0x12-prefixed frames.
    pub new_format: bool,
    pub can_print_label: bool,
    pub label_value: String,
    pub back_paper_num: i32,
    pub detection_value: i32,
    pub gray_print: bool,
    pub gray_thin_eneragy: u32,
    pub gray_moderation_eneragy: u32,
    pub gray_deepen_eneragy: u32,
    pub slow_interval: u32,
    pub gray_image_speed: u32,
    pub gray_scale: f64,
    pub show_electricity_model: i32,
    pub add_mor_pix: bool,
    #[serde(rename = "A4XII")]
    pub a4xii: bool,
    pub tattoo_paper: bool,
    pub tattoo_speed: u32,
    pub tattoo_energy: u32,
    pub add_more_pix_num: i32,
    pub d1key: String,
    pub lzo_version: String,
    pub label_device: bool,
    pub auto_label_check: bool,
    pub core_print: bool,
    pub label_up_paper_num: i32,
    pub left_margin: i32,
}

#[derive(Deserialize)]
struct CatalogFile {
    models: Vec<Model>,
}

static CATALOG: OnceLock<Vec<Model>> = OnceLock::new();

pub fn models() -> &'static [Model] {
    CATALOG.get_or_init(|| {
        let f: CatalogFile =
            serde_json::from_str(include_str!("../models.json")).expect("embedded catalogue");
        f.models
    })
}

/// Model for a Bluetooth device name, or `None` if the name is not a known printer.
pub fn lookup(ble_name: &str) -> Option<&'static Model> {
    models()
        .iter()
        .filter(|m| !m.model_no.is_empty() && ble_name.starts_with(&m.model_no))
        .max_by_key(|m| m.model_no.len())
}

impl Model {
    /// Raster width in pixels, before any left padding.
    pub fn image_width(&self) -> u32 {
        if self.add_mor_pix {
            self.paper_size
        } else {
            self.print_size
        }
    }

    /// Blank pixels the A4 heads expect on the left of each row.
    pub fn left_pixels(&self) -> u32 {
        if self.size != 8 || !self.add_mor_pix {
            return 0;
        }
        if self.add_more_pix_num >= 0 {
            self.add_more_pix_num as u32
        } else if self.model_no.starts_with("X8-W") || self.model_no.starts_with("X8-L") {
            40
        } else {
            64
        }
    }

    pub fn is_a4(&self) -> bool {
        self.size == 8
    }

    /// Printable width in millimetres.
    pub fn printable_mm(&self) -> f64 {
        self.image_width() as f64 / self.devdpi.max(1) as f64 * 25.4
    }

    /// Roll (or sheet) width in millimetres, as printed on the paper box.
    pub fn paper_mm(&self) -> u32 {
        let p = self.printable_mm();
        match p.round() as u32 {
            44..=50 => 58,
            51..=56 => 57,
            70..=76 => 80,
            100..=112 => 112,
            190..=220 => 210,
            n if n <= 20 => 15,
            n => n,
        }
    }

    /// LZO row blocks are used when the firmware is at least `lzo_version`.
    pub fn uses_lzo(&self, firmware: Option<&str>) -> bool {
        if self.core_print {
            return true;
        }
        match self.lzo_version.as_str() {
            "" | "0" => false,
            "1" => true,
            min => firmware.is_some_and(|fw| compare_versions(fw, min).is_gt()),
        }
    }
}

/// Compares dotted versions numerically; non-digit suffixes are ignored.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..pa.len().max(pb.len()) {
        let o = pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0));
        if o.is_ne() {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_loads() {
        assert!(models().len() > 100);
    }

    #[test]
    fn longest_prefix_wins() {
        let m = lookup("SC03h-2F95").unwrap();
        assert_eq!(m.model_no, "SC03h");
        assert!(m.use_spp);
        assert_eq!(m.image_width(), 384);
        assert_eq!(m.paper_mm(), 58);
        assert!(lookup("NotAPrinter").is_none());
    }

    #[test]
    fn versions() {
        assert!(compare_versions("2.0.67", "2.0.66").is_gt());
        assert!(compare_versions("2.0.66", "2.0.66").is_eq());
        assert!(compare_versions("1.9", "2.0.66").is_lt());
    }
}
