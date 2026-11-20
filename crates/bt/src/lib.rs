//! Discovery and connection to iPrint-compatible printers.
//!
//! Every printer advertises over BLE, which is how it is discovered. Data then
//! goes either over BLE GATT or, for models flagged `useSPP`, over classic
//! Bluetooth RFCOMM at the same address. On Android both go through the
//! app's Kotlin plugin (see [`android`]).

#[cfg(target_os = "android")]
pub mod android;
#[cfg(not(target_os = "android"))]
mod ble;
mod printer;

// Classic Bluetooth (RFCOMM) per platform, same API: `open(&Found)` and `Rfcomm`.
#[cfg(windows)]
#[path = "win.rs"]
mod classic;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod classic;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod classic;

pub use printer::{Info, Printer};

use raketo_core::Model;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Bluetooth adapter")]
    NoAdapter,
    #[error("printer not found")]
    NotFound,
    #[error("pairing failed: {0}")]
    Pairing(String),
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("write failed: {0}")]
    Write(String),
    #[error("no reply from printer")]
    Timeout,
    #[error("classic Bluetooth is not supported on this platform yet")]
    Unsupported,
    #[cfg(not(target_os = "android"))]
    #[error(transparent)]
    Ble(#[from] btleplug::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub name: String,
    /// 48-bit Bluetooth address.
    pub address: u64,
    pub rssi: Option<i16>,
    #[serde(skip)]
    pub model: &'static Model,
}

impl Found {
    pub fn address_string(&self) -> String {
        format_address(self.address)
    }
}

pub fn format_address(a: u64) -> String {
    (0..6).rev().map(|i| format!("{:02X}", (a >> (i * 8)) & 0xFF)).collect::<Vec<_>>().join(":")
}

pub fn parse_address(s: &str) -> Option<u64> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return None;
    }
    u64::from_str_radix(&hex, 16).ok()
}

/// Scans for known printers for `timeout`.
pub async fn scan(timeout: Duration) -> Result<Vec<Found>> {
    #[cfg(target_os = "android")]
    return android::scan(timeout).await;
    #[cfg(not(target_os = "android"))]
    ble::scan(timeout).await
}

/// Connects using the transport the model requires.
pub async fn connect(found: &Found) -> Result<Printer> {
    Printer::connect(found).await
}
