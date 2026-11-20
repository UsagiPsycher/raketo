//! Sheets planned ahead and printed one after the other.
//!
//! Each entry is one sheet (copies count as separate sheets). Between sheets
//! the queue either waits until the user has torn off the last one, or keeps
//! going without a pause, optionally with a dashed cut line in between.

use super::{Error, PrintRequest, Result, Service, ServiceEvent, Work};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::imageops::FilterType;
use raketo_core::JobOptions;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use tokio::sync::Notify;

/// How sheets follow each other.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueueMode {
    /// Waits for the user to tear off each sheet before printing the next.
    pub pause: bool,
    /// Without pauses, prints a dashed cut line between sheets.
    pub separators: bool,
}

impl Default for QueueMode {
    fn default() -> Self {
        QueueMode { pause: true, separators: true }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub id: u64,
    pub title: String,
    /// Tool that made it ("photo", "todo"…), for the icon.
    pub kind: String,
    /// Small PNG data URL of the first page.
    pub thumb: String,
    pub copies: u32,
    /// Copies already printed.
    pub done: u32,
    pub length_mm: u32,
    pub label: bool,
    pub source: String,
    /// Unchecked sheets stay in the queue but are skipped.
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    pub items: Vec<QueueEntry>,
    pub mode: QueueMode,
    pub running: bool,
    /// Waiting for the user to tear off the last sheet.
    pub waiting: bool,
    /// Entry being printed.
    pub current: Option<u64>,
    /// Service job id of the sheet being printed, for progress events.
    pub job: Option<u64>,
    pub error: Option<String>,
    /// Sheets printed since the queue was started.
    pub printed: u32,
}

struct Item {
    entry: QueueEntry,
    pages: Vec<Vec<u8>>,
    options: JobOptions,
}

#[derive(Default)]
struct Inner {
    items: Vec<Item>,
    running: bool,
    waiting: bool,
    current: Option<u64>,
    job: Option<u64>,
    error: Option<String>,
    printed: u32,
    next_id: u64,
}

#[derive(Default)]
pub(crate) struct Queue {
    inner: StdMutex<Inner>,
    wake: Notify,
    go: AtomicBool,
    stop: AtomicBool,
}

fn thumbnail(img: &image::DynamicImage) -> String {
    let w = 120u32.min(img.width().max(1));
    let h = ((img.height() as u64 * w as u64) / img.width().max(1) as u64).clamp(1, 600) as u32;
    let t = img.resize_exact(w, h, FilterType::Triangle).to_luma8();
    let mut out = Vec::new();
    if image::DynamicImage::ImageLuma8(t).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).is_err() {
        return String::new();
    }
    format!("data:image/png;base64,{}", B64.encode(out))
}

impl Service {
    pub fn queue_state(&self) -> QueueState {
        let q = self.0.queue.inner.lock().unwrap();
        QueueState {
            items: q.items.iter().map(|i| i.entry.clone()).collect(),
            mode: self.settings().queue,
            running: q.running,
            waiting: q.waiting,
            current: q.current,
            job: q.job,
            error: q.error.clone(),
            printed: q.printed,
        }
    }

    fn queue_changed(&self) {
        self.0.events.send(ServiceEvent::Queue(self.queue_state())).ok();
    }

    fn queue_edit<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        let r = f(&mut self.0.queue.inner.lock().unwrap());
        self.queue_changed();
        r
    }

    /// Adds a sheet at the end of the queue and returns its id.
    pub fn queue_add(&self, req: PrintRequest, title: &str, kind: &str) -> Result<u64> {
        let imgs = Self::decode(&req.pages)?;
        let first = imgs.first().ok_or_else(|| Error::Image("nothing to print".into()))?;
        let model = self.model(None);
        let w = model.image_width() as f64;
        let dots: f64 = imgs.iter().map(|i| i.height() as f64 * w / i.width().max(1) as f64).sum();
        let length_mm = (dots / model.devdpi.max(1) as f64 * 25.4).round() as u32;
        let thumb = thumbnail(first);
        let copies = req.options.copies.clamp(1, 50);
        let label = req.options.label;
        Ok(self.queue_edit(|q| {
            q.next_id += 1;
            let id = q.next_id;
            q.items.push(Item {
                entry: QueueEntry {
                    id,
                    title: title.chars().take(80).collect(),
                    kind: kind.into(),
                    thumb,
                    copies,
                    done: 0,
                    length_mm,
                    label,
                    source: req.source,
                    selected: true,
                },
                pages: req.pages,
                options: req.options,
            });
            id
        }))
    }

    /// Removes a sheet that is not being printed.
    pub fn queue_remove(&self, id: u64) {
        self.queue_edit(|q| {
            let current = q.current;
            q.items.retain(|i| i.entry.id != id || current == Some(id));
        });
    }

    /// Moves a sheet to `index`; the sheet being printed stays first.
    pub fn queue_move(&self, id: u64, index: usize) {
        self.queue_edit(|q| {
            let Some(from) = q.items.iter().position(|i| i.entry.id == id) else { return };
            let floor = usize::from(q.current.is_some_and(|c| q.items.first().map(|i| i.entry.id) == Some(c)));
            if from < floor {
                return;
            }
            let item = q.items.remove(from);
            let to = index.clamp(floor, q.items.len());
            q.items.insert(to, item);
        });
    }

    /// Empties the queue, except the sheet being printed.
    pub fn queue_clear(&self) {
        self.queue_edit(|q| {
            let current = q.current;
            q.items.retain(|i| current == Some(i.entry.id));
            q.error = None;
        });
    }

    /// Checks or unchecks one sheet, or all of them without `id`.
    pub fn queue_select(&self, id: Option<u64>, selected: bool) {
        self.queue_edit(|q| {
            for i in q.items.iter_mut().filter(|i| id.is_none_or(|id| i.entry.id == id)) {
                i.entry.selected = selected;
            }
        });
    }

    pub fn set_queue_mode(&self, mode: QueueMode) {
        self.update_settings(|s| s.queue = mode);
        self.queue_changed();
        // A queue waiting for a tear goes on by itself once pauses are off.
        self.0.queue.wake.notify_waiters();
    }

    /// Starts printing the queue from the first sheet.
    pub fn queue_start(&self) -> Result<()> {
        if self.state().printer.is_none() {
            return Err(Error::NotConnected);
        }
        let start = self.queue_edit(|q| {
            if q.running || !q.items.iter().any(|i| i.entry.selected) {
                return false;
            }
            q.running = true;
            q.error = None;
            q.printed = 0;
            true
        });
        if start {
            self.0.queue.stop.store(false, Ordering::SeqCst);
            let svc = self.clone();
            tokio::spawn(async move { svc.run_queue().await });
        }
        Ok(())
    }

    /// Prints the next sheet after a pause.
    pub fn queue_continue(&self) {
        self.0.queue.go.store(true, Ordering::SeqCst);
        self.0.queue.wake.notify_waiters();
    }

    /// Stops after the sheet being printed.
    pub fn queue_stop(&self) {
        self.0.queue.stop.store(true, Ordering::SeqCst);
        self.0.queue.wake.notify_waiters();
    }

    fn stopping(&self) -> bool {
        self.0.queue.stop.load(Ordering::SeqCst)
    }

    /// Waits for "next sheet", a stop, or pauses being turned off.
    async fn wait_for_tear(&self) {
        let q = &self.0.queue;
        q.go.store(false, Ordering::SeqCst);
        self.queue_edit(|q| q.waiting = true);
        loop {
            let woken = q.wake.notified();
            if q.go.load(Ordering::SeqCst) || self.stopping() || !self.settings().queue.pause {
                break;
            }
            woken.await;
        }
        self.queue_edit(|q| q.waiting = false);
    }

    async fn feed_out(&self) {
        let Some(info) = self.state().printer else { return };
        let Some(model) = raketo_core::lookup(&info.model_no) else { return };
        let id = self.new_job_id();
        let r = vec![raketo_core::encode::blank(model)];
        self.run_locked(id, Work::Raster(r), JobOptions::default(), false, false).await.ok();
    }

    async fn run_queue(&self) {
        let mut printed = false;
        // The last sheet was printed without feeding the paper out.
        let mut tight = false;
        let mut prev_label = false;
        let mut tear = false;
        loop {
            if self.stopping() {
                break;
            }
            let next = {
                let q = self.0.queue.inner.lock().unwrap();
                let mut chosen = q.items.iter().filter(|i| i.entry.selected);
                chosen.next().map(|i| {
                    let more = i.entry.done + 1 < i.entry.copies || chosen.next().is_some();
                    (i.entry.id, i.pages.clone(), i.options.clone(), more)
                })
            };
            let Some((id, pages, opts, more)) = next else { break };
            if tear && self.settings().queue.pause {
                if tight {
                    self.feed_out().await;
                    tight = false;
                }
                self.wait_for_tear().await;
                tear = false;
                // Sheets may have been removed or moved meanwhile: pick again.
                continue;
            }
            let mode = self.settings().queue;
            let model = self.model(None);
            let label = opts.label && model.can_print_label;
            if printed && !mode.pause && mode.separators && !label && !prev_label {
                let sep = vec![raketo_core::encode::separator(model)];
                let o = JobOptions { feed_after: false, content: raketo_core::Content::Text, ..Default::default() };
                if let Err(e) = self.run_locked(self.new_job_id(), Work::Raster(sep), o, false, false).await {
                    self.queue_edit(|q| q.error = Some(e.to_string()));
                    break;
                }
            }
            let back_to_back = !mode.pause && more && !label;
            let sheet = JobOptions { copies: 1, feed_after: !back_to_back, ..opts };
            let imgs = match Self::decode(&pages) {
                Ok(i) => i,
                Err(e) => {
                    self.queue_edit(|q| q.error = Some(e.to_string()));
                    break;
                }
            };
            let job = self.new_job_id();
            self.queue_edit(|q| {
                q.current = Some(id);
                q.job = Some(job);
            });
            let res = self.run_locked(job, Work::Images(imgs), sheet, true, false).await;
            let failed = self.queue_edit(|q| {
                q.current = None;
                q.job = None;
                match &res {
                    Ok(()) => {
                        q.printed += 1;
                        if let Some(pos) = q.items.iter().position(|i| i.entry.id == id) {
                            let e = &mut q.items[pos].entry;
                            e.done += 1;
                            if e.done >= e.copies {
                                q.items.remove(pos);
                            }
                        }
                        false
                    }
                    Err(e) => {
                        q.error = Some(e.to_string());
                        true
                    }
                }
            });
            if failed {
                break;
            }
            printed = true;
            tear = true;
            tight = back_to_back;
            prev_label = label;
        }
        if tight {
            self.feed_out().await;
        }
        self.queue_edit(|q| {
            q.running = false;
            q.waiting = false;
            q.current = None;
            q.job = None;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_luma8(8, 8).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn unchecked_sheets_stay_but_are_not_counted() {
        let dir = std::env::temp_dir().join(format!("raketo-queue-{}", std::process::id()));
        let svc = Service::new(Some(dir.clone()));
        let req = || PrintRequest { pages: vec![png()], options: JobOptions::default(), source: "local".into() };
        let a = svc.queue_add(req(), "a", "photo").unwrap();
        let b = svc.queue_add(req(), "b", "photo").unwrap();
        assert!(svc.queue_state().items.iter().all(|i| i.selected));
        svc.queue_select(Some(a), false);
        let s = svc.queue_state();
        assert_eq!(s.items.len(), 2);
        assert!(!s.items[0].selected && s.items[1].selected);
        svc.queue_select(None, false);
        assert!(svc.queue_state().items.iter().all(|i| !i.selected));
        svc.queue_select(Some(b), true);
        assert!(svc.queue_state().items[1].selected);
        std::fs::remove_dir_all(dir).ok();
    }
}
