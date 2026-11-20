#[cfg(not(target_os = "android"))]
use crate::ble::BleLink;
use crate::{Error, Found, Result};
use raketo_core::frame::{self, cmd};
use raketo_core::notify::{Event, Parser, Status};
use raketo_core::Model;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Notify};

enum Link {
    #[cfg(not(target_os = "android"))]
    Ble(BleLink),
    #[cfg(target_os = "android")]
    Android(crate::android::Conn),
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    Spp(Arc<crate::classic::Rfcomm>),
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub name: String,
    pub address: String,
    pub model_no: String,
    pub firmware: Option<String>,
    pub dev_type: Option<u8>,
    pub status: Option<Status>,
    pub transport: &'static str,
    pub paper_mm: u32,
    pub width_px: u32,
    pub dpi: u32,
    pub gray: bool,
    pub label: bool,
}

struct Flow {
    paused: AtomicBool,
    resume: Notify,
    /// Remaining window credit (YMS-BT01), or -1 when unused.
    credit: AtomicI32,
    credit_added: Notify,
}

pub struct Printer {
    pub found: Found,
    link: Link,
    flow: Arc<Flow>,
    events: broadcast::Sender<Event>,
    firmware: std::sync::Mutex<Option<String>>,
    alive: Arc<AtomicBool>,
}

impl Printer {
    pub async fn connect(found: &Found) -> Result<Printer> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let alive = Arc::new(AtomicBool::new(true));
        #[cfg(target_os = "android")]
        let link = Link::Android(crate::android::Conn::open(found, tx, alive.clone()).await?);
        #[cfg(not(target_os = "android"))]
        let link = if found.model.use_spp {
            Self::open_spp(found, tx, alive.clone()).await?
        } else {
            Link::Ble(BleLink::open(found, tx).await?)
        };
        let (events, _) = broadcast::channel(64);
        let flow = Arc::new(Flow {
            paused: AtomicBool::new(false),
            resume: Notify::new(),
            credit: AtomicI32::new(-1),
            credit_added: Notify::new(),
        });
        let (ev, fl, model) = (events.clone(), flow.clone(), found.model);
        tokio::spawn(async move {
            let mut parser = Parser::default();
            while let Some(bytes) = rx.recv().await {
                for e in parser.push(&bytes, model) {
                    match &e {
                        Event::FlowPause => fl.paused.store(true, Ordering::SeqCst),
                        Event::FlowResume => {
                            fl.paused.store(false, Ordering::SeqCst);
                            fl.resume.notify_waiters();
                        }
                        Event::Window { init, credit } => {
                            if *init {
                                fl.credit.store(*credit as i32, Ordering::SeqCst);
                            } else {
                                fl.credit.fetch_add(*credit as i32, Ordering::SeqCst);
                            }
                            fl.credit_added.notify_waiters();
                        }
                        _ => {}
                    }
                    ev.send(e).ok();
                }
            }
        });
        Ok(Printer { found: found.clone(), link, flow, events, firmware: Default::default(), alive })
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    async fn open_spp(found: &Found, tx: mpsc::UnboundedSender<Vec<u8>>, alive: Arc<AtomicBool>) -> Result<Link> {
        let f = found.clone();
        let sock = tokio::task::spawn_blocking(move || crate::classic::open(&f))
        .await
        .map_err(|e| Error::Connect(e.to_string()))??;
        let reader = sock.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while alive.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(Some(n)) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        alive.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
        });
        Ok(Link::Spp(sock))
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos", target_os = "android")))]
    async fn open_spp(_: &Found, _: mpsc::UnboundedSender<Vec<u8>>, _: Arc<AtomicBool>) -> Result<Link> {
        Err(Error::Unsupported)
    }

    pub fn model(&self) -> &'static Model {
        self.found.model
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn firmware(&self) -> Option<String> {
        self.firmware.lock().unwrap().clone()
    }

    fn chunk_size(&self) -> usize {
        match &self.link {
            #[cfg(target_os = "android")]
            Link::Android(c) => c.chunk_size(),
            #[cfg(not(target_os = "android"))]
            Link::Ble(b) => b.chunk_size(),
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            Link::Spp(_) => self.model().img_mtu.max(20) as usize,
        }
    }

    async fn raw_write(&self, data: &[u8]) -> Result<()> {
        match &self.link {
            #[cfg(target_os = "android")]
            Link::Android(c) => c.write(data).await,
            #[cfg(not(target_os = "android"))]
            Link::Ble(b) => b.write(data).await,
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            Link::Spp(s) => {
                let (s, d) = (s.clone(), data.to_vec());
                tokio::task::spawn_blocking(move || s.write_all(&d))
                    .await
                    .map_err(|e| Error::Write(e.to_string()))?
            }
        }
    }

    /// Sends a command and waits for the first event matching `want`.
    async fn request<T>(&self, data: Vec<u8>, want: impl Fn(&Event) -> Option<T>) -> Result<T> {
        let mut rx = self.subscribe();
        self.raw_write(&data).await?;
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(e) => {
                        if let Some(t) = want(&e) {
                            return Some(t);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return None,
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(3), fut).await.ok().flatten().ok_or(Error::Timeout)
    }

    pub async fn status(&self) -> Result<Status> {
        self.request(frame::status(), |e| if let Event::Status(s) = e { Some(s.clone()) } else { None }).await
    }

    /// Connection handshake: device info, anti-clone challenge when the model has one, status.
    pub async fn handshake(&self) -> Result<Info> {
        let m = self.model();
        if !m.d1key.is_empty() {
            let challenge: [u8; 8] = rand::random();
            self.raw_write(&frame::frame(cmd::CHALLENGE, &challenge)).await.ok();
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let info_cmd = if m.new_format { frame::frame_prefixed(cmd::DEV_INFO, &[0]) } else { frame::dev_info() };
        let dev = self
            .request(info_cmd, |e| match e {
                Event::DevInfo { dev_type, firmware, .. } => Some((*dev_type, firmware.clone())),
                _ => None,
            })
            .await
            .ok();
        if let Some((_, fw)) = &dev {
            *self.firmware.lock().unwrap() = Some(fw.clone());
        }
        let status = self.status().await.ok();
        Ok(self.info(dev, status))
    }

    fn info(&self, dev: Option<(u8, String)>, status: Option<Status>) -> Info {
        let m = self.model();
        Info {
            name: self.found.name.clone(),
            address: self.found.address_string(),
            model_no: m.model_no.clone(),
            firmware: dev.as_ref().map(|d| d.1.clone()),
            dev_type: dev.map(|d| d.0),
            status,
            transport: if m.use_spp { "spp" } else { "ble" },
            paper_mm: m.paper_mm(),
            width_px: m.image_width(),
            dpi: m.devdpi,
            gray: m.gray_print,
            label: m.can_print_label,
        }
    }

    /// Streams a job, honouring the packet interval and printer flow control.
    pub async fn send(&self, job: &[u8], progress: impl Fn(usize, usize)) -> Result<()> {
        let m = self.model();
        let interval = match &self.link {
            #[cfg(target_os = "android")]
            Link::Android(c) if c.spp => m.interval.clamp(2, 10),
            #[cfg(target_os = "android")]
            Link::Android(_) => m.interval,
            #[cfg(not(target_os = "android"))]
            Link::Ble(_) => m.interval,
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            Link::Spp(_) => m.interval.clamp(2, 10),
        } as u64;
        let windowed = m.model_no == "YMS-BT01" && job.len() > 100;
        let mut sent = 0;
        while sent < job.len() {
            if !self.is_alive() {
                return Err(Error::Write("connection lost".into()));
            }
            while self.flow.paused.load(Ordering::SeqCst) {
                let _ = tokio::time::timeout(Duration::from_millis(500), self.flow.resume.notified()).await;
            }
            if windowed {
                while self.flow.credit.load(Ordering::SeqCst) == 0 {
                    let _ = tokio::time::timeout(Duration::from_millis(500), self.flow.credit_added.notified()).await;
                }
                if self.flow.credit.load(Ordering::SeqCst) > 0 {
                    self.flow.credit.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let n = self.chunk_size().min(job.len() - sent);
            self.raw_write(&job[sent..sent + n]).await?;
            sent += n;
            progress(sent, job.len());
            if interval > 0 {
                tokio::time::sleep(Duration::from_millis(interval)).await;
            }
        }
        Ok(())
    }

    pub async fn disconnect(&self) {
        self.alive.store(false, Ordering::Relaxed);
        match &self.link {
            #[cfg(target_os = "android")]
            Link::Android(c) => c.close().await,
            #[cfg(not(target_os = "android"))]
            Link::Ble(b) => b.close().await,
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            Link::Spp(s) => s.close(),
        }
    }
}
