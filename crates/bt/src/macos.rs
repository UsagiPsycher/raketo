//! macOS classic Bluetooth through IOBluetooth.
//!
//! CoreBluetooth never exposes MAC addresses, so the classic device is found
//! by name (paired devices first, then an inquiry). IOBluetooth delivers RFCOMM
//! events on the run loop of the thread that opened the channel, so a
//! dedicated thread owns the channel and serves writes from a queue.
//! macOS pairs on its own when the channel opens.

use crate::{Error, Found, Result};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_foundation::{NSArray, NSString};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[link(name = "IOBluetooth", kind = "framework")]
extern "C" {}

const IO_RETURN_SUCCESS: i32 = 0;

struct Ivars {
    tx: Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "RaketoRfcommDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn on_data(&self, _channel: *mut AnyObject, data: *mut c_void, length: usize) {
            if !data.is_null() && length > 0 {
                let b = unsafe { std::slice::from_raw_parts(data as *const u8, length) }.to_vec();
                self.ivars().tx.send(b).ok();
            }
        }

        #[unsafe(method(rfcommChannelClosed:))]
        fn on_closed(&self, _channel: *mut AnyObject) {
            self.ivars().closed.store(true, Ordering::SeqCst);
        }
    }
);

impl Delegate {
    fn new(ivars: Ivars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).ok_or_else(|| Error::Connect("IOBluetooth unavailable".into()))
}

/// Lets IOBluetooth deliver pending events on this thread.
fn pump(seconds: f64) {
    unsafe {
        let Some(loop_cls) = AnyClass::get(c"NSRunLoop") else { return };
        let Some(date_cls) = AnyClass::get(c"NSDate") else { return };
        let rl: Retained<AnyObject> = msg_send![loop_cls, currentRunLoop];
        let until: Retained<AnyObject> = msg_send![date_cls, dateWithTimeIntervalSinceNow: seconds];
        let _: () = msg_send![&*rl, runUntilDate: &*until];
    }
}

fn device_name(d: &AnyObject) -> String {
    unsafe {
        let n: Option<Retained<NSString>> = msg_send![d, name];
        n.map(|s| s.to_string()).unwrap_or_default()
    }
}

fn address_string(a: u64) -> String {
    (0..6).rev().map(|i| format!("{:02x}", (a >> (i * 8)) & 0xFF)).collect::<Vec<_>>().join("-")
}

fn find_device(found: &Found) -> Result<Retained<AnyObject>> {
    let dev_cls = class(c"IOBluetoothDevice")?;
    unsafe {
        if found.address != 0 {
            let s = NSString::from_str(&address_string(found.address));
            let d: Option<Retained<AnyObject>> = msg_send![dev_cls, deviceWithAddressString: &*s];
            if let Some(d) = d {
                return Ok(d);
            }
        }
        let paired: Option<Retained<NSArray<AnyObject>>> = msg_send![dev_cls, pairedDevices];
        if let Some(list) = paired {
            for d in list.iter() {
                if device_name(&d) == found.name {
                    return Ok(d);
                }
            }
        }
        // Not paired yet: run a classic inquiry and match the advertised name.
        let inq_cls = class(c"IOBluetoothDeviceInquiry")?;
        let inq: Option<Retained<AnyObject>> = msg_send![inq_cls, inquiryWithDelegate: std::ptr::null_mut::<AnyObject>()];
        let inq = inq.ok_or_else(|| Error::Connect("inquiry".into()))?;
        let _: () = msg_send![&*inq, setInquiryLength: 8u8];
        let _: () = msg_send![&*inq, setUpdateNewDeviceNames: Bool::YES];
        let r: i32 = msg_send![&*inq, start];
        if r != IO_RETURN_SUCCESS {
            return Err(Error::Connect(format!("inquiry failed ({r:#x})")));
        }
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            pump(0.25);
            let list: Option<Retained<NSArray<AnyObject>>> = msg_send![&*inq, foundDevices];
            if let Some(list) = list {
                for d in list.iter() {
                    if device_name(&d) == found.name {
                        let _: i32 = msg_send![&*inq, stop];
                        return Ok(d);
                    }
                }
            }
        }
        let _: i32 = msg_send![&*inq, stop];
        Err(Error::NotFound)
    }
}

enum Cmd {
    Write(Vec<u8>, SyncSender<Result<()>>),
    Close,
}

pub struct Rfcomm {
    cmd: Mutex<Sender<Cmd>>,
    rx: Mutex<(Receiver<Vec<u8>>, Vec<u8>)>,
    closed: Arc<AtomicBool>,
}

/// Opens the serial port channel on a dedicated run-loop thread.
pub fn open(found: &Found) -> Result<Arc<Rfcomm>> {
    let (data_tx, data_rx) = channel::<Vec<u8>>();
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (ready_tx, ready_rx) = sync_channel::<Result<()>>(1);
    let closed = Arc::new(AtomicBool::new(false));
    let found = found.clone();
    let closed2 = closed.clone();
    std::thread::spawn(move || {
        let res = (|| -> Result<(Retained<AnyObject>, Retained<AnyObject>, Retained<Delegate>)> {
            let dev = find_device(&found)?;
            let delegate = Delegate::new(Ivars { tx: data_tx, closed: closed2.clone() });
            for ch in [1u8, 2, 3] {
                let mut channel: *mut AnyObject = std::ptr::null_mut();
                let r: i32 = unsafe {
                    msg_send![&*dev, openRFCOMMChannelSync: &mut channel, withChannelID: ch, delegate: &*delegate]
                };
                if r == IO_RETURN_SUCCESS && !channel.is_null() {
                    let channel = unsafe { Retained::retain(channel) }.ok_or_else(|| Error::Connect("channel".into()))?;
                    return Ok((dev, channel, delegate));
                }
            }
            Err(Error::Connect("RFCOMM channel refused".into()))
        })();
        let (dev, chan, _delegate) = match res {
            Ok(v) => {
                ready_tx.send(Ok(())).ok();
                v
            }
            Err(e) => {
                ready_tx.send(Err(e)).ok();
                return;
            }
        };
        let mtu: u16 = unsafe { msg_send![&*chan, getMTU] };
        let mtu = mtu.max(20) as usize;
        loop {
            pump(0.01);
            match cmd_rx.try_recv() {
                Ok(Cmd::Write(data, reply)) => {
                    let mut ok = Ok(());
                    for chunk in data.chunks(mtu) {
                        let r: i32 = unsafe { msg_send![&*chan, writeSync: chunk.as_ptr() as *mut c_void, length: chunk.len() as u16] };
                        if r != IO_RETURN_SUCCESS {
                            ok = Err(Error::Write(format!("IOReturn {r:#x}")));
                            break;
                        }
                    }
                    reply.send(ok).ok();
                }
                Ok(Cmd::Close) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if closed2.load(Ordering::SeqCst) {
                break;
            }
        }
        unsafe {
            let _: i32 = msg_send![&*chan, closeChannel];
            let _: i32 = msg_send![&*dev, closeConnection];
        }
        closed2.store(true, Ordering::SeqCst);
    });
    ready_rx.recv_timeout(Duration::from_secs(30)).map_err(|_| Error::Timeout)??;
    Ok(Arc::new(Rfcomm { cmd: Mutex::new(cmd_tx), rx: Mutex::new((data_rx, Vec::new())), closed }))
}

impl Rfcomm {
    pub fn write_all(&self, data: &[u8]) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::Write("closed".into()));
        }
        let (tx, rx) = sync_channel(1);
        self.cmd.lock().unwrap().send(Cmd::Write(data.to_vec(), tx)).map_err(|_| Error::Write("closed".into()))?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| Error::Timeout)?
    }

    /// Returns `Ok(None)` on timeout, `Err` when the link is gone.
    pub fn read(&self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut g = self.rx.lock().unwrap();
        if g.1.is_empty() {
            match g.0.recv_timeout(Duration::from_millis(200)) {
                Ok(b) => g.1 = b,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return if self.closed.load(Ordering::SeqCst) { Err(Error::Connect("closed by printer".into())) } else { Ok(None) };
                }
                Err(_) => return Err(Error::Connect("closed".into())),
            }
        }
        let n = buf.len().min(g.1.len());
        buf[..n].copy_from_slice(&g.1[..n]);
        g.1.drain(..n);
        Ok(Some(n))
    }

    pub fn close(&self) {
        self.cmd.lock().unwrap().send(Cmd::Close).ok();
    }
}
