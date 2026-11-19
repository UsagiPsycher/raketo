//! Linux classic Bluetooth: pairing through BlueZ (D-Bus), RFCOMM through a
//! native AF_BLUETOOTH socket.

use crate::{Error, Result};
use std::sync::Arc;
use std::time::Duration;

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_RFCOMM: libc::c_int = 3;

#[repr(C)]
struct SockaddrRc {
    rc_family: libc::sa_family_t,
    /// Address bytes in little-endian order, as BlueZ expects.
    rc_bdaddr: [u8; 6],
    rc_channel: u8,
}

fn bd_addr(address: u64) -> bluer::Address {
    let b = address.to_be_bytes();
    bluer::Address::new([b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Pairs the printer if BlueZ does not know it yet. A local agent accepts
/// "just works" confirmations and offers PIN 0000, so no desktop dialog is needed.
pub fn ensure_paired(address: u64) -> Result<()> {
    let handle = tokio::runtime::Handle::current();
    handle.block_on(async move {
        let session = bluer::Session::new().await.map_err(|e| Error::Pairing(e.to_string()))?;
        let adapter = session.default_adapter().await.map_err(|_| Error::NoAdapter)?;
        adapter.set_powered(true).await.ok();
        let device = adapter.device(bd_addr(address)).map_err(|e| Error::Pairing(e.to_string()))?;
        if device.is_paired().await.unwrap_or(false) {
            return Ok(());
        }
        let agent = bluer::agent::Agent {
            request_default: false,
            request_pin_code: Some(Box::new(|_| Box::pin(async { Ok("0000".to_string()) }))),
            request_passkey: Some(Box::new(|_| Box::pin(async { Ok(0) }))),
            request_confirmation: Some(Box::new(|_| Box::pin(async { Ok(()) }))),
            request_authorization: Some(Box::new(|_| Box::pin(async { Ok(()) }))),
            authorize_service: Some(Box::new(|_| Box::pin(async { Ok(()) }))),
            ..Default::default()
        };
        let _agent = session.register_agent(agent).await.map_err(|e| Error::Pairing(e.to_string()))?;
        // The device must be known to BlueZ: a short discovery makes it appear.
        if device.name().await.is_err() {
            let _disc = adapter.discover_devices().await.ok();
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
        match device.pair().await {
            Ok(()) => {}
            Err(e) if e.kind == bluer::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::Pairing(e.to_string())),
        }
        device.set_trusted(true).await.ok();
        Ok(())
    })
}

pub struct Rfcomm {
    fd: libc::c_int,
}

impl Drop for Rfcomm {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

/// Pairs if needed, then opens the serial port channel.
pub fn open(found: &crate::Found) -> Result<Arc<Rfcomm>> {
    // Some printers accept RFCOMM without pairing; pair only when refused.
    match Rfcomm::open(found.address) {
        Ok(s) => Ok(s),
        Err(_) => {
            ensure_paired(found.address)?;
            Rfcomm::open(found.address)
        }
    }
}

fn os_err(what: &str) -> Error {
    Error::Connect(format!("{what}: {}", std::io::Error::last_os_error()))
}

impl Rfcomm {
    fn connect_channel(address: u64, channel: u8) -> Result<libc::c_int> {
        unsafe {
            let fd = libc::socket(AF_BLUETOOTH, libc::SOCK_STREAM, BTPROTO_RFCOMM);
            if fd < 0 {
                return Err(os_err("socket"));
            }
            let b = address.to_le_bytes();
            let addr = SockaddrRc { rc_family: AF_BLUETOOTH as libc::sa_family_t, rc_bdaddr: [b[0], b[1], b[2], b[3], b[4], b[5]], rc_channel: channel };
            let r = libc::connect(fd, &addr as *const SockaddrRc as *const libc::sockaddr, std::mem::size_of::<SockaddrRc>() as libc::socklen_t);
            if r < 0 {
                let e = os_err("connect");
                libc::close(fd);
                return Err(e);
            }
            Ok(fd)
        }
    }

    /// Serial port service is on channel 1 on these printers; 2 and 3 are tried as fallbacks.
    pub fn open(address: u64) -> Result<Arc<Rfcomm>> {
        let mut last = None;
        for ch in [1u8, 2, 3] {
            match Self::connect_channel(address, ch) {
                Ok(fd) => {
                    let tv = libc::timeval { tv_sec: 0, tv_usec: 200_000 };
                    unsafe {
                        libc::setsockopt(
                            fd,
                            libc::SOL_SOCKET,
                            libc::SO_RCVTIMEO,
                            &tv as *const libc::timeval as *const libc::c_void,
                            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
                        );
                    }
                    return Ok(Arc::new(Rfcomm { fd }));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Connect("no RFCOMM channel".into())))
    }

    pub fn write_all(&self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let n = unsafe { libc::send(self.fd, data.as_ptr() as *const libc::c_void, data.len(), libc::MSG_NOSIGNAL) };
            if n <= 0 {
                return Err(Error::Write(std::io::Error::last_os_error().to_string()));
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    /// Returns `Ok(None)` on timeout, `Err` when the link is gone.
    pub fn read(&self, buf: &mut [u8]) -> Result<Option<usize>> {
        let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n > 0 {
            return Ok(Some(n as usize));
        }
        if n == 0 {
            return Err(Error::Connect("closed by printer".into()));
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EAGAIN) | Some(libc::EINTR) => Ok(None),
            _ => Err(Error::Connect(e.to_string())),
        }
    }

    pub fn close(&self) {
        unsafe {
            libc::shutdown(self.fd, libc::SHUT_RDWR);
        }
    }
}
