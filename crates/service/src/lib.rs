//! Printer service shared by the desktop app and the LAN server.
//!
//! Holds the connection, serialises print jobs (one at a time, any source),
//! keeps per-printer settings and broadcasts state changes.

use image::{DynamicImage, ImageFormat};
use raketo_bt::{Found, Info, Printer};
use raketo_core::encode::GrayCurve;
use raketo_core::notify::{Event, Status};
use raketo_core::{JobOptions, Model};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

mod queue;
pub use queue::{QueueEntry, QueueMode, QueueState};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no printer connected")]
    NotConnected,
    #[error("printer not found, scan again")]
    UnknownPrinter,
    #[error("unreadable image: {0}")]
    Image(String),
    #[error("printer problem: {0}")]
    Printer(String),
    #[error(transparent)]
    Bt(#[from] raketo_bt::Error),
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A printer seen during the last scan.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub name: String,
    pub address: String,
    pub rssi: Option<i16>,
    pub model_no: String,
    pub paper_mm: u32,
    pub transport: &'static str,
}

/// Per-printer settings, keyed by Bluetooth address.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrinterSettings {
    pub gray_curve: Option<GrayCurve>,
    pub density: Option<u8>,
    /// Paper sensor baseline learnt while printing labels.
    pub label_baseline: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub last_address: Option<String>,
    pub last_name: Option<String>,
    pub printers: HashMap<String, PrinterSettings>,
    pub share_on_lan: bool,
    pub lan_port: Option<u16>,
    pub lan_name: Option<String>,
    pub queue: QueueMode,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            last_address: None,
            last_name: None,
            printers: HashMap::new(),
            // Sharing is on unless the user turns it off.
            share_on_lan: true,
            lan_port: None,
            lan_name: None,
            queue: QueueMode::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub connecting: bool,
    pub printer: Option<Info>,
    pub status: Option<Status>,
    pub busy: bool,
    pub queue: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServiceEvent {
    State(State),
    Progress { job: u64, sent: usize, total: usize },
    JobDone { job: u64, ok: bool, error: Option<String> },
    Queue(QueueState),
}

/// Pages and options of a print request, as sent by any client.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintRequest {
    /// Encoded images (PNG, JPEG…), one per page.
    #[serde(skip)]
    pub pages: Vec<Vec<u8>>,
    #[serde(default)]
    pub options: JobOptions,
    /// Who sent the job, shown in the queue ("this computer", a LAN address…).
    #[serde(default)]
    pub source: String,
}

struct Inner {
    printer: Mutex<Option<Arc<Printer>>>,
    state: StdMutex<State>,
    seen: StdMutex<HashMap<String, Found>>,
    settings: StdMutex<Settings>,
    settings_path: Option<PathBuf>,
    events: broadcast::Sender<ServiceEvent>,
    job_lock: Mutex<()>,
    next_job: AtomicU64,
    queue: queue::Queue,
}

#[derive(Clone)]
pub struct Service(Arc<Inner>);

impl Service {
    /// `config_dir` holds `settings.json`; `None` keeps settings in memory only.
    pub fn new(config_dir: Option<PathBuf>) -> Service {
        let settings_path = config_dir.map(|d| d.join("settings.json"));
        let settings = settings_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (events, _) = broadcast::channel(256);
        let svc = Service(Arc::new(Inner {
            printer: Mutex::new(None),
            state: StdMutex::new(State { connecting: false, printer: None, status: None, busy: false, queue: 0 }),
            seen: Default::default(),
            settings: StdMutex::new(settings),
            settings_path,
            events,
            job_lock: Mutex::new(()),
            next_job: AtomicU64::new(1),
            queue: Default::default(),
        }));
        svc
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServiceEvent> {
        self.0.events.subscribe()
    }

    pub fn state(&self) -> State {
        self.0.state.lock().unwrap().clone()
    }

    fn update(&self, f: impl FnOnce(&mut State)) {
        let s = {
            let mut st = self.0.state.lock().unwrap();
            f(&mut st);
            st.clone()
        };
        self.0.events.send(ServiceEvent::State(s)).ok();
    }

    pub fn settings(&self) -> Settings {
        self.0.settings.lock().unwrap().clone()
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        let s = {
            let mut st = self.0.settings.lock().unwrap();
            f(&mut st);
            st.clone()
        };
        if let Some(p) = &self.0.settings_path {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            std::fs::write(p, serde_json::to_vec_pretty(&s).unwrap_or_default()).ok();
        }
        s
    }

    pub async fn scan(&self, seconds: u64) -> Result<Vec<Seen>> {
        let found = raketo_bt::scan(Duration::from_secs(seconds.clamp(2, 20))).await?;
        let mut seen = self.0.seen.lock().unwrap();
        let mut out = Vec::new();
        for f in found {
            out.push(Seen {
                name: f.name.clone(),
                address: f.address_string(),
                rssi: f.rssi,
                model_no: f.model.model_no.clone(),
                paper_mm: f.model.paper_mm(),
                transport: if f.model.use_spp { "spp" } else { "ble" },
            });
            seen.insert(f.address_string(), f);
        }
        Ok(out)
    }

    /// Connects to a scanned printer, or to a known address and name without scanning.
    pub async fn connect(&self, address: &str, name: Option<&str>) -> Result<Info> {
        let found = {
            let seen = self.0.seen.lock().unwrap();
            seen.get(address).cloned()
        };
        let found = match (found, name) {
            (Some(f), _) => f,
            (None, Some(n)) => {
                let model = raketo_core::lookup(n).ok_or(Error::UnknownPrinter)?;
                let address = raketo_bt::parse_address(address).ok_or(Error::UnknownPrinter)?;
                Found { name: n.to_string(), address, rssi: None, model }
            }
            (None, None) => return Err(Error::UnknownPrinter),
        };
        self.disconnect().await;
        self.update(|s| s.connecting = true);
        let res = async {
            let p = Printer::connect(&found).await?;
            let info = p.handshake().await?;
            Ok::<_, Error>((p, info))
        }
        .await;
        match res {
            Ok((p, info)) => {
                let p = Arc::new(p);
                self.watch_events(&p);
                *self.0.printer.lock().await = Some(p);
                self.update(|s| {
                    s.connecting = false;
                    s.status = info.status.clone();
                    s.printer = Some(info.clone());
                });
                self.update_settings(|s| {
                    s.last_address = Some(info.address.clone());
                    s.last_name = Some(info.name.clone());
                });
                Ok(info)
            }
            Err(e) => {
                self.update(|s| s.connecting = false);
                Err(e)
            }
        }
    }

    /// Reconnects to the last printer, if any.
    pub async fn reconnect_last(&self) -> Result<Info> {
        let s = self.settings();
        match (s.last_address, s.last_name) {
            (Some(a), Some(n)) => self.connect(&a, Some(&n)).await,
            _ => Err(Error::UnknownPrinter),
        }
    }

    pub async fn disconnect(&self) {
        if let Some(p) = self.0.printer.lock().await.take() {
            p.disconnect().await;
        }
        self.update(|s| {
            s.printer = None;
            s.status = None;
        });
    }

    fn watch_events(&self, p: &Arc<Printer>) {
        let mut rx = p.subscribe();
        let svc = self.clone();
        tokio::spawn(async move {
            while let Ok(e) = rx.recv().await {
                if let Event::Status(st) = e {
                    svc.update(|s| s.status = Some(st));
                }
            }
        });
    }

    /// Starts the background status poll; call from inside the Tokio runtime.
    pub fn start(&self) {
        let svc = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let p = svc.0.printer.lock().await.clone();
                let Some(p) = p else { continue };
                if !p.is_alive() {
                    svc.disconnect().await;
                    continue;
                }
                if let Ok(_g) = svc.0.job_lock.try_lock() {
                    p.status().await.ok();
                }
            }
        });
    }

    fn model(&self, model_no: Option<&str>) -> &'static Model {
        let connected = self.state().printer.and_then(|i| raketo_core::lookup(&i.model_no));
        connected
            .or_else(|| model_no.and_then(raketo_core::lookup))
            .unwrap_or_else(|| raketo_core::lookup("SC03h").expect("catalogue"))
    }

    /// Applies stored per-printer settings to request options.
    fn effective_options(&self, mut o: JobOptions) -> JobOptions {
        if let Some(info) = self.state().printer {
            let set = self.settings().printers.get(&info.address).cloned().unwrap_or_default();
            if o.gray_curve.is_none() {
                o.gray_curve = set.gray_curve;
            }
            o.firmware = info.firmware.clone();
        }
        o
    }

    fn decode(pages: &[Vec<u8>]) -> Result<Vec<DynamicImage>> {
        pages.iter().map(|b| image::load_from_memory(b).map_err(|e| Error::Image(e.to_string()))).collect()
    }

    /// Exact dots the printer will print, as PNG, one per page.
    pub fn preview(&self, pages: &[Vec<u8>], opts: JobOptions, model_no: Option<&str>) -> Result<Vec<Vec<u8>>> {
        let model = self.model(model_no);
        let imgs = Self::decode(pages)?;
        let opts = self.effective_options(opts);
        let raster = raketo_core::rasterize(model, &imgs, &opts);
        raster
            .iter()
            .map(|r| {
                let mut img = r.preview();
                if let (Some(c), raketo_core::Raster::Gray(g)) = (opts.gray_curve, r) {
                    // Show the calibrated levels the way the printer renders them.
                    let mut g = g.clone();
                    g.levels.iter_mut().for_each(|l| *l = c.apply(*l));
                    img = g.to_image();
                }
                let mut out = Vec::new();
                img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| Error::Image(e.to_string()))?;
                Ok(out)
            })
            .collect()
    }

    /// Queues a job and returns its id; progress and completion come as events.
    pub fn print(&self, req: PrintRequest) -> Result<u64> {
        let imgs = Self::decode(&req.pages)?;
        if imgs.is_empty() {
            return Err(Error::Image("nothing to print".into()));
        }
        Ok(self.enqueue(Work::Images(imgs), req.options, true))
    }

    /// Prints 16 raw gray steps (no tone curve, no calibration) to measure the printer.
    pub fn print_gray_wedge(&self) -> Result<u64> {
        let info = self.state().printer.ok_or(Error::NotConnected)?;
        let model = raketo_core::lookup(&info.model_no).ok_or(Error::UnknownPrinter)?;
        if !model.gray_print {
            return Err(Error::Printer("no_gray".into()));
        }
        let w = model.image_width();
        let h = 48u32;
        let mut levels = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let tick = y < 6 && x % (w / 16) == 0;
                levels[(y * w + x) as usize] = if tick { 15 } else { (x * 16 / w) as u8 };
            }
        }
        let g = raketo_core::raster::GrayBitmap { width: w, height: h, levels };
        let opts = JobOptions { gray: true, ..Default::default() };
        Ok(self.enqueue(Work::Raster(vec![raketo_core::Raster::Gray(g)]), opts, false))
    }

    fn enqueue(&self, work: Work, opts: JobOptions, calibrated: bool) -> u64 {
        let id = self.new_job_id();
        let svc = self.clone();
        self.update(|s| s.queue += 1);
        tokio::spawn(async move {
            svc.run_locked(id, work, opts, calibrated, true).await.ok();
        });
        id
    }

    fn new_job_id(&self) -> u64 {
        self.0.next_job.fetch_add(1, Ordering::SeqCst)
    }

    /// Runs one job once the printer is free, and reports its end.
    async fn run_locked(&self, id: u64, work: Work, opts: JobOptions, calibrated: bool, counted: bool) -> Result<()> {
        let _guard = self.0.job_lock.lock().await;
        self.update(|s| {
            if counted {
                s.queue = s.queue.saturating_sub(1);
            }
            s.busy = true;
        });
        let res = self.run_job(id, work, opts, calibrated).await;
        self.update(|s| s.busy = false);
        self.0
            .events
            .send(ServiceEvent::JobDone { job: id, ok: res.is_ok(), error: res.as_ref().err().map(|e| e.to_string()) })
            .ok();
        res
    }

    async fn run_job(&self, id: u64, work: Work, opts: JobOptions, calibrated: bool) -> Result<()> {
        let p = self.0.printer.lock().await.clone().ok_or(Error::NotConnected)?;
        if !p.is_alive() {
            self.disconnect().await;
            return Err(Error::NotConnected);
        }
        let mut opts = self.effective_options(opts);
        if !calibrated {
            opts.gray_curve = None;
        }
        if let Ok(st) = p.status().await {
            if let Some(problem) = st.problem() {
                if problem != "low_battery" {
                    return Err(Error::Printer(problem.into()));
                }
            }
        }
        let model = p.model();
        let label = opts.label && model.can_print_label;
        let copies = opts.copies.max(1);
        let bytes = tokio::task::spawn_blocking({
            // Label paper prints one copy per label, with a gap search in between.
            let opts = JobOptions { copies: if label { 1 } else { opts.copies }, ..opts.clone() };
            move || {
                let r = match work {
                    Work::Images(imgs) => raketo_core::rasterize(model, &imgs, &opts),
                    Work::Raster(r) => r,
                };
                raketo_core::build_job(model, &r, &opts)
            }
        })
        .await
        .map_err(|e| Error::Image(e.to_string()))?;
        if label {
            return self.run_label_job(id, &p, &bytes, copies, &opts).await;
        }
        let events = self.0.events.clone();
        let last = std::sync::atomic::AtomicUsize::new(0);
        p.send(&bytes, |sent, total| {
            // Throttle progress events to ~2% steps.
            if sent == total || sent - last.load(Ordering::Relaxed) >= total / 50 + 1 {
                last.store(sent, Ordering::Relaxed);
                events.send(ServiceEvent::Progress { job: id, sent, total }).ok();
            }
        })
        .await?;
        Ok(())
    }
}

impl Service {
    async fn run_label_job(&self, id: u64, p: &Arc<Printer>, job: &[u8], copies: u32, opts: &JobOptions) -> Result<()> {
        let model = p.model();
        let address = p.found.address_string();
        let baseline = self.settings().printers.get(&address).and_then(|s| s.label_baseline);
        let mut det = raketo_core::encode::LabelDetector::new(model, baseline);
        // Each probe moves 8 lines (about 1 mm). Two label heights always
        // cross a gap; the app searches 8× longer, which wastes paper on failure.
        let attempts = (opts.label_height_mm.unwrap_or(40) * 2 + 10).clamp(20, 400);
        let total = job.len() * copies as usize;
        for n in 0..copies {
            self.find_label(p, &mut det, attempts).await?;
            let done = job.len() * n as usize;
            let events = self.0.events.clone();
            p.send(job, |sent, _| {
                events.send(ServiceEvent::Progress { job: id, sent: done + sent, total }).ok();
            })
            .await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        // Leave the next label edge at the tear bar.
        self.find_label(p, &mut det, attempts).await.ok();
        if let Some(b) = det.learnt {
            self.update_settings(|s| s.printers.entry(address).or_default().label_baseline = Some(b));
        }
        Ok(())
    }

    /// Feeds 8 lines at a time, reading the paper sensor, until a label edge shows up.
    async fn find_label(&self, p: &Arc<Printer>, det: &mut raketo_core::encode::LabelDetector, attempts: u32) -> Result<()> {
        let model = p.model();
        if model.auto_label_check {
            p.send(&raketo_core::encode::label_auto_check(), |_, _| {}).await?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            return Ok(());
        }
        for _ in 0..attempts {
            let mut rx = p.subscribe();
            p.send(&raketo_core::encode::label_probe(8), |_, _| {}).await?;
            let mut found = false;
            let mut readings = 0;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while readings < 8 {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Ok(Event::Status(st))) => {
                        readings += 1;
                        if let Some(problem) = st.problem() {
                            if problem != "low_battery" {
                                return Err(Error::Printer(problem.into()));
                            }
                        }
                        if let Some(r) = st.sensor {
                            found |= det.push(r);
                        }
                    }
                    Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                    _ => break,
                }
            }
            if readings == 0 {
                return Err(Error::Printer("no_sensor".into()));
            }
            if found {
                p.send(&raketo_core::encode::label_advance(model), |_, _| {}).await?;
                tokio::time::sleep(Duration::from_millis(200)).await;
                return Ok(());
            }
        }
        Err(Error::Printer("label_not_found".into()))
    }
}

enum Work {
    Images(Vec<DynamicImage>),
    Raster(Vec<raketo_core::Raster>),
}
