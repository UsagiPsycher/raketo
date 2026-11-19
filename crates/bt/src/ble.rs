use crate::{Error, Found, Result};
use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

const fn u16_uuid(v: u16) -> Uuid {
    Uuid::from_u128(((v as u128) << 96) | 0x0000_0000_0000_1000_8000_0080_5F9B_34FB)
}

/// Write characteristics tried by the app, in order.
const WRITE: [Uuid; 5] = [
    u16_uuid(0xAE01),
    u16_uuid(0xFF02),
    u16_uuid(0xAB01),
    Uuid::from_u128(0x49535343_8841_43F4_A8D4_ECBE34729BB3),
    u16_uuid(0xAE81),
];
/// Notify characteristics (AE02 is used by many clones besides the app's list).
const NOTIFY: [Uuid; 5] = [u16_uuid(0xAE02), u16_uuid(0xAE04), u16_uuid(0xAB03), u16_uuid(0xFF03), u16_uuid(0xAE82)];

pub async fn adapter() -> Result<Adapter> {
    let m = Manager::new().await?;
    m.adapters().await?.into_iter().next().ok_or(Error::NoAdapter)
}

fn addr_u64(p: &btleplug::api::BDAddr) -> u64 {
    p.into_inner().iter().fold(0u64, |a, &b| (a << 8) | b as u64)
}

pub async fn scan(timeout: Duration) -> Result<Vec<Found>> {
    let a = adapter().await?;
    let mut events = a.events().await?;
    a.start_scan(ScanFilter::default()).await?;
    let mut found: HashMap<u64, Found> = HashMap::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let ev = tokio::time::timeout_at(deadline, events.next()).await;
        let Ok(Some(ev)) = ev else { break };
        let id = match ev {
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => id,
            _ => continue,
        };
        let Ok(p) = a.peripheral(&id).await else { continue };
        let Ok(Some(props)) = p.properties().await else { continue };
        let Some(name) = props.local_name else { continue };
        let Some(model) = raketo_core::lookup(&name) else { continue };
        let address = addr_u64(&props.address);
        found.insert(address, Found { name, address, rssi: props.rssi, model });
    }
    a.stop_scan().await.ok();
    let mut v: Vec<Found> = found.into_values().collect();
    v.sort_by_key(|f| std::cmp::Reverse(f.rssi.unwrap_or(i16::MIN)));
    Ok(v)
}

pub struct BleLink {
    peripheral: Peripheral,
    write: Characteristic,
    chunk: std::sync::atomic::AtomicUsize,
}

impl BleLink {
    pub async fn open(found: &Found, incoming: mpsc::UnboundedSender<Vec<u8>>) -> Result<BleLink> {
        let a = adapter().await?;
        let mut target = None;
        // The peripheral is usually known from the scan; rescan briefly otherwise.
        for attempt in 0..2 {
            for p in a.peripherals().await? {
                if p.properties().await.ok().flatten().is_some_and(|pr| addr_u64(&pr.address) == found.address) {
                    target = Some(p);
                }
            }
            if target.is_some() || attempt == 1 {
                break;
            }
            a.start_scan(ScanFilter::default()).await?;
            tokio::time::sleep(Duration::from_secs(4)).await;
            a.stop_scan().await.ok();
        }
        let p = target.ok_or(Error::NotFound)?;
        let mut last = None;
        for _ in 0..3 {
            match p.connect().await {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        if let Some(e) = last {
            return Err(Error::Connect(e.to_string()));
        }
        p.discover_services().await?;
        let chars = p.characteristics();
        let write = WRITE
            .iter()
            .find_map(|u| chars.iter().find(|c| c.uuid == *u))
            .or_else(|| chars.iter().find(|c| c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)))
            .cloned()
            .ok_or_else(|| Error::Connect("no write characteristic".into()))?;
        for c in chars.iter().filter(|c| {
            c.properties.contains(CharPropFlags::NOTIFY) && (NOTIFY.contains(&c.uuid) || c.service_uuid == write.service_uuid)
        }) {
            p.subscribe(c).await.ok();
        }
        let mut stream = p.notifications().await?;
        tokio::spawn(async move {
            while let Some(n) = stream.next().await {
                if incoming.send(n.value).is_err() {
                    break;
                }
            }
        });
        let model = found.model;
        let chunk = if model.can_change_mtu { model.img_mtu.max(23) as usize - 3 } else { 20 };
        Ok(BleLink { peripheral: p, write, chunk: chunk.into() })
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn write(&self, data: &[u8]) -> Result<()> {
        match self.peripheral.write(&self.write, data, WriteType::WithoutResponse).await {
            Ok(()) => Ok(()),
            // The negotiated MTU may be smaller than the model's: fall back to 20-byte packets.
            Err(_) if data.len() > 20 => {
                self.chunk.store(20, std::sync::atomic::Ordering::Relaxed);
                for c in data.chunks(20) {
                    self.peripheral
                        .write(&self.write, c, WriteType::WithoutResponse)
                        .await
                        .map_err(|e| Error::Write(e.to_string()))?;
                }
                Ok(())
            }
            Err(e) => Err(Error::Write(e.to_string())),
        }
    }

    pub async fn close(&self) {
        self.peripheral.disconnect().await.ok();
    }
}
