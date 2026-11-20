//! Android: Bluetooth goes through the app's Kotlin plugin.
//!
//! The app installs a [`Bridge`] at startup. Scans, connection and writes are
//! blocking calls into Kotlin made from worker threads; bytes from the printer
//! come back through [`on_data`], called from Kotlin over JNI.

use crate::{Error, Found, Result};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// What the Android side provides. Calls block until done.
pub trait Bridge: Send + Sync {
    /// BLE scan: name, address ("AA:BB:CC:DD:EE:FF") and signal strength of each device seen.
    fn scan(&self, seconds: u32) -> std::result::Result<Vec<(String, String, Option<i16>)>, String>;
    /// Connects over classic SPP or BLE GATT and returns the packet size to use.
    fn open(&self, address: &str, spp: bool, mtu: u16) -> std::result::Result<usize, String>;
    fn write(&self, data: &[u8]) -> std::result::Result<(), String>;
    fn close(&self);
}

static BRIDGE: OnceLock<Arc<dyn Bridge>> = OnceLock::new();
type Incoming = (mpsc::UnboundedSender<Vec<u8>>, Arc<AtomicBool>);
static INCOMING: Mutex<Option<Incoming>> = Mutex::new(None);

pub fn set_bridge(b: Arc<dyn Bridge>) {
    BRIDGE.set(b).ok();
}

/// Bytes received from the printer.
pub fn on_data(bytes: Vec<u8>) {
    if let Some((tx, _)) = INCOMING.lock().unwrap().as_ref() {
        tx.send(bytes).ok();
    }
}

/// The connection dropped on the Android side.
pub fn on_closed() {
    if let Some((_, alive)) = INCOMING.lock().unwrap().as_ref() {
        alive.store(false, Ordering::Relaxed);
    }
}

fn bridge() -> Result<Arc<dyn Bridge>> {
    BRIDGE.get().cloned().ok_or(Error::NoAdapter)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::Connect(e.to_string()))
}

pub async fn scan(timeout: Duration) -> Result<Vec<Found>> {
    let b = bridge()?;
    let secs = timeout.as_secs().max(1) as u32;
    let seen = blocking(move || b.scan(secs)).await?.map_err(Error::Connect)?;
    let mut found: Vec<Found> = Vec::new();
    for (name, addr, rssi) in seen {
        let (Some(model), Some(address)) = (raketo_core::lookup(&name), crate::parse_address(&addr)) else { continue };
        if !found.iter().any(|f| f.address == address) {
            found.push(Found { name, address, rssi, model });
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.rssi.unwrap_or(i16::MIN)));
    Ok(found)
}

pub struct Conn {
    bridge: Arc<dyn Bridge>,
    chunk: AtomicUsize,
    pub spp: bool,
}

impl Conn {
    pub async fn open(found: &Found, incoming: mpsc::UnboundedSender<Vec<u8>>, alive: Arc<AtomicBool>) -> Result<Conn> {
        let b = bridge()?;
        *INCOMING.lock().unwrap() = Some((incoming, alive));
        let m = found.model;
        let spp = m.use_spp;
        let mtu = if spp || !m.can_change_mtu { 23 } else { m.img_mtu.max(23) as u16 };
        let (b2, addr) = (b.clone(), found.address_string());
        let chunk = blocking(move || b2.open(&addr, spp, mtu)).await?.map_err(Error::Connect)?;
        let chunk = if spp { m.img_mtu.max(20) as usize } else { chunk.max(20) };
        Ok(Conn { bridge: b, chunk: chunk.into(), spp })
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk.load(Ordering::Relaxed)
    }

    pub async fn write(&self, data: &[u8]) -> Result<()> {
        let (b, d) = (self.bridge.clone(), data.to_vec());
        blocking(move || b.write(&d)).await?.map_err(Error::Write)
    }

    pub async fn close(&self) {
        let b = self.bridge.clone();
        blocking(move || b.close()).await.ok();
        INCOMING.lock().unwrap().take();
    }
}
