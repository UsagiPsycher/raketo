//! Windows classic Bluetooth: pairing through WinRT, RFCOMM through Winsock.

use crate::{Error, Result};
use std::sync::Arc;
use windows::core::GUID;
use windows::Devices::Bluetooth::BluetoothDevice;
use windows::Devices::Enumeration::{
    DeviceInformationCustomPairing, DevicePairingKinds, DevicePairingProtectionLevel, DevicePairingRequestedEventArgs,
    DevicePairingResultStatus,
};
use windows::Foundation::TypedEventHandler;
use windows::Win32::Devices::Bluetooth::{AF_BTH, BTHPROTO_RFCOMM, SOCKADDR_BTH};
use windows::Win32::Networking::WinSock::{
    closesocket, connect, recv, send, setsockopt, socket, WSAGetLastError, WSAStartup, INVALID_SOCKET, SEND_RECV_FLAGS,
    SOCKADDR, SOCKET, SOCKET_ERROR, SOCK_STREAM, SOL_SOCKET, SO_RCVTIMEO, WSADATA,
};

const SPP: GUID = GUID::from_u128(0x00001101_0000_1000_8000_00805F9B34FB);

/// Pairs the classic side of the printer if Windows does not know it yet.
/// These printers use "just works" pairing; a PIN of 0000 is offered if asked.
pub fn ensure_paired(address: u64) -> Result<()> {
    let dev = BluetoothDevice::FromBluetoothAddressAsync(address)
        .and_then(|op| op.get())
        .map_err(|e| Error::Pairing(e.message().to_string()))?;
    let info = dev.DeviceInformation().map_err(|e| Error::Pairing(e.message().to_string()))?;
    let pairing = info.Pairing().map_err(|e| Error::Pairing(e.message().to_string()))?;
    if pairing.IsPaired().unwrap_or(false) {
        return Ok(());
    }
    let custom: DeviceInformationCustomPairing = pairing.Custom().map_err(|e| Error::Pairing(e.message().to_string()))?;
    let handler = TypedEventHandler::<DeviceInformationCustomPairing, DevicePairingRequestedEventArgs>::new(|_, args| {
        if let Some(a) = args.as_ref() {
            if a.PairingKind()? == DevicePairingKinds::ProvidePin {
                a.AcceptWithPin(&windows::core::HSTRING::from("0000"))?;
            } else {
                a.Accept()?;
            }
        }
        Ok(())
    });
    let token = custom.PairingRequested(&handler).map_err(|e| Error::Pairing(e.message().to_string()))?;
    let kinds = DevicePairingKinds::ConfirmOnly
        | DevicePairingKinds::ProvidePin
        | DevicePairingKinds::ConfirmPinMatch
        | DevicePairingKinds::DisplayPin;
    let res = custom
        .PairWithProtectionLevelAsync(kinds, DevicePairingProtectionLevel::Default)
        .and_then(|op| op.get());
    custom.RemovePairingRequested(token).ok();
    let status = res.and_then(|r| r.Status()).map_err(|e| Error::Pairing(e.message().to_string()))?;
    match status {
        DevicePairingResultStatus::Paired | DevicePairingResultStatus::AlreadyPaired => Ok(()),
        s => Err(Error::Pairing(format!("status {}", s.0))),
    }
}

/// Pairs if needed, then opens the serial port channel.
pub fn open(found: &crate::Found) -> Result<Arc<Rfcomm>> {
    ensure_paired(found.address)?;
    Rfcomm::open(found.address)
}

pub struct Rfcomm {
    sock: SOCKET,
}

// Winsock sockets can be used from several threads (one reader, one writer).
unsafe impl Send for Rfcomm {}
unsafe impl Sync for Rfcomm {}

fn wsa_err(what: &str) -> Error {
    let code = unsafe { WSAGetLastError() };
    Error::Connect(format!("{what}: WSA error {}", code.0))
}

impl Rfcomm {
    /// Connects to the SPP service; falls back to channel 1 like the app does.
    pub fn open(address: u64) -> Result<Arc<Rfcomm>> {
        unsafe {
            let mut data = WSADATA::default();
            WSAStartup(0x0202, &mut data);
        }
        let attempt = |port: u32, service: GUID| -> Result<SOCKET> {
            unsafe {
                let s = socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32)
                    .map_err(|e| Error::Connect(e.message().to_string()))?;
                if s == INVALID_SOCKET {
                    return Err(wsa_err("socket"));
                }
                let addr = SOCKADDR_BTH { addressFamily: AF_BTH, btAddr: address, serviceClassId: service, port };
                let r = connect(
                    s,
                    &addr as *const SOCKADDR_BTH as *const SOCKADDR,
                    std::mem::size_of::<SOCKADDR_BTH>() as i32,
                );
                if r == SOCKET_ERROR {
                    let e = wsa_err("connect");
                    closesocket(s);
                    return Err(e);
                }
                Ok(s)
            }
        };
        let sock = attempt(0, SPP).or_else(|_| attempt(1, GUID::zeroed()))?;
        unsafe {
            let ms: u32 = 200;
            setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, Some(&ms.to_ne_bytes()));
        }
        Ok(Arc::new(Rfcomm { sock }))
    }

    pub fn write_all(&self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let n = unsafe { send(self.sock, data, SEND_RECV_FLAGS(0)) };
            if n == SOCKET_ERROR || n <= 0 {
                return Err(Error::Write(format!("WSA error {}", unsafe { WSAGetLastError() }.0)));
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    /// Returns `Ok(None)` on timeout, `Err` when the link is gone.
    pub fn read(&self, buf: &mut [u8]) -> Result<Option<usize>> {
        let n = unsafe { recv(self.sock, buf, SEND_RECV_FLAGS(0)) };
        if n > 0 {
            return Ok(Some(n as usize));
        }
        if n == 0 {
            return Err(Error::Connect("closed by printer".into()));
        }
        let code = unsafe { WSAGetLastError() }.0;
        // WSAETIMEDOUT
        if code == 10060 {
            Ok(None)
        } else {
            Err(Error::Connect(format!("WSA error {code}")))
        }
    }

    pub fn close(&self) {
        unsafe {
            closesocket(self.sock);
        }
    }
}
