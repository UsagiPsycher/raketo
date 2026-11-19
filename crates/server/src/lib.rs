//! LAN sharing of the connected printer.
//!
//! One port serves the web page (same UI as the desktop app, print-only), its
//! JSON API, and an IPP Everywhere endpoint so other computers and phones can
//! add the printer without a driver. Both are advertised over mDNS.

#[allow(dead_code)]
mod ipp;
mod raster;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State as AxState};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use futures::Stream;
use ipp::{op, status, tag, Group, Message, Value};
use raketo_core::{Content, JobOptions};
use raketo_service::{PrintRequest, Service, ServiceEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(rust_embed::Embed)]
#[folder = "../../dist"]
struct Assets;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub port: u16,
    pub name: String,
    /// Addresses other devices can open in a browser.
    pub urls: Vec<String>,
    /// IPP URI to add the printer manually.
    pub ipp: Vec<String>,
    /// Stable name on the local network, e.g. `http://raketo.local/`.
    pub local_url: Option<String>,
    /// Set by the host app when the OS firewall still blocks the port.
    pub firewall_blocked: bool,
}

pub struct Server {
    stop: Option<oneshot::Sender<()>>,
    mdns: Option<mdns_sd::ServiceDaemon>,
    pub info: ShareInfo,
}

impl Server {
    pub fn stop(mut self) {
        if let Some(s) = self.stop.take() {
            s.send(()).ok();
        }
        if let Some(m) = self.mdns.take() {
            m.shutdown().ok();
        }
    }
}

#[derive(Clone)]
struct App {
    svc: Service,
    name: String,
    uuid: String,
    started: Instant,
    jobs: Arc<Mutex<Jobs>>,
}

#[derive(Default)]
struct Jobs {
    next: i32,
    list: HashMap<i32, IppJob>,
    /// Service job id → IPP job id.
    by_service: HashMap<u64, i32>,
}

#[derive(Clone)]
struct IppJob {
    id: i32,
    name: String,
    user: String,
    /// IPP job-state: 3 pending, 5 processing, 7 canceled, 8 aborted, 9 completed.
    state: i32,
    reason: &'static str,
    created: u64,
    pages: i32,
    quality: i32,
    copies: i32,
}

pub async fn start(svc: Service, port: u16, name: String) -> std::io::Result<Server> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
    let port = listener.local_addr()?.port();
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, format!("raketo.{host}.{name}").as_bytes()).to_string();
    let _ = &host;
    let app = App { svc: svc.clone(), name: name.clone(), uuid: uuid.clone(), started: Instant::now(), jobs: Default::default() };
    spawn_job_tracker(app.clone());

    let router = Router::new()
        .route("/ipp/print", post(ipp_handler))
        .route("/ipp/print/", post(ipp_handler))
        .route("/api/state", get(api_state))
        .route("/api/settings", get(api_settings))
        .route("/api/models", get(api_models))
        .route("/api/preview", post(api_preview))
        .route("/api/print", post(api_print))
        .route("/api/events", get(api_events))
        .fallback(get(static_file))
        .layer(DefaultBodyLimit::max(96 * 1024 * 1024))
        .with_state(app);

    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async {
                rx.await.ok();
            })
            .await
            .ok();
    });

    let ips = lan_ips();
    let suffix = if port == 80 { String::new() } else { format!(":{port}") };
    let urls = ips.iter().map(|ip| format!("http://{ip}{suffix}/")).collect();
    let ipp_uris = ips.iter().map(|ip| format!("ipp://{ip}:{port}/ipp/print")).collect();
    let (mdns, local) = match advertise(&name, port, &ips, &uuid, &svc) {
        Ok((d, h)) => (Some(d), Some(h)),
        Err(_) => (None, None),
    };
    let local_url = local.map(|h| format!("http://{}{suffix}/", h.trim_end_matches('.')));
    Ok(Server { stop: Some(tx), mdns, info: ShareInfo { port, name, urls, ipp: ipp_uris, local_url, firewall_blocked: false } })
}

/// LAN addresses, the default-route one first; virtual adapters (WSL, Hyper-V, VMs, VPN taps) are skipped.
fn lan_ips() -> Vec<IpAddr> {
    let virtual_name = |n: &str| {
        let n = n.to_lowercase();
        ["vethernet", "wsl", "hyper-v", "virtualbox", "vmware", "docker", "loopback", "tap", "tun", "zerotier", "tailscale"]
            .iter()
            .any(|k| n.contains(k))
    };
    let mut ips: Vec<IpAddr> = Vec::new();
    if let Ok(ip) = local_ip_address::local_ip() {
        ips.push(ip);
    }
    if let Ok(list) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in list {
            if ip.is_ipv4() && !ip.is_loopback() && !is_link_local(&ip) && !virtual_name(&name) && !ips.contains(&ip) {
                ips.push(ip);
            }
        }
    }
    ips
}

fn is_link_local(ip: &IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if v4.is_link_local())
}

/// First free name among raketo.local, raketo-2.local… (another computer may run Raketo).
fn pick_host_name(d: &mdns_sd::ServiceDaemon, ours: &[IpAddr]) -> String {
    for n in 1..=9 {
        let name = if n == 1 { "raketo.local.".to_string() } else { format!("raketo-{n}.local.") };
        let Ok(rx) = d.resolve_hostname(&name, Some(1200)) else { return name };
        let deadline = Instant::now() + std::time::Duration::from_millis(1500);
        let mut taken = false;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(mdns_sd::HostnameResolutionEvent::AddressesFound(_, addrs)) => {
                    if addrs.iter().any(|a| !ours.contains(a)) {
                        taken = true;
                        break;
                    }
                }
                Ok(mdns_sd::HostnameResolutionEvent::SearchTimeout(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        d.stop_resolve_hostname(&name).ok();
        if !taken {
            return name;
        }
    }
    "raketo-host.local.".into()
}

fn advertise(name: &str, port: u16, ips: &[IpAddr], uuid: &str, svc: &Service) -> Result<(mdns_sd::ServiceDaemon, String), mdns_sd::Error> {
    let d = mdns_sd::ServiceDaemon::new()?;
    let host_name = pick_host_name(&d, ips);
    let model = svc.state().printer.map(|p| p.model_no).unwrap_or_else(|| "thermal".into());
    let ipp_txt: Vec<(&str, String)> = vec![
        ("txtvers", "1".into()),
        ("qtotal", "1".into()),
        ("rp", "ipp/print".into()),
        ("ty", format!("Raketo {model}")),
        ("product", "(Raketo)".into()),
        ("note", "".into()),
        ("pdl", "image/pwg-raster,image/urf,image/jpeg,image/png".into()),
        ("URF", "V1.4,CP1,W8,RS200".into()),
        ("Color", "F".into()),
        ("Duplex", "F".into()),
        ("UUID", uuid.into()),
        ("adminurl", format!("http://{}:{port}/", host_name.trim_end_matches('.'))),
        ("priority", "50".into()),
        ("kind", "roll,receipt,label".into()),
        ("PaperMax", "<legal-A4".into()),
        ("TLS", "".into()),
    ];
    let txt: Vec<(&str, &str)> = ipp_txt.iter().filter(|(k, _)| *k != "TLS").map(|(k, v)| (*k, v.as_str())).collect();
    let ipp = mdns_sd::ServiceInfo::new("_ipp._tcp.local.", name, &host_name, ips, port, &txt[..])?;
    d.register(ipp)?;
    // Subtypes let IPP Everywhere / AirPrint clients find it.
    for sub in ["_universal._sub._ipp._tcp.local.", "_print._sub._ipp._tcp.local."] {
        if let Ok(s) = mdns_sd::ServiceInfo::new(sub, name, &host_name, ips, port, &txt[..]) {
            d.register(s).ok();
        }
    }
    let web = mdns_sd::ServiceInfo::new("_http._tcp.local.", name, &host_name, ips, port, &[("path", "/")][..])?;
    d.register(web)?;
    Ok((d, host_name))
}

fn spawn_job_tracker(app: App) {
    let mut rx = app.svc.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let mut jobs = app.jobs.lock().unwrap();
            match ev {
                ServiceEvent::Progress { job, .. } => {
                    if let Some(id) = jobs.by_service.get(&job).copied() {
                        if let Some(j) = jobs.list.get_mut(&id) {
                            j.state = 5;
                            j.reason = "job-printing";
                        }
                    }
                }
                ServiceEvent::JobDone { job, ok, .. } => {
                    if let Some(id) = jobs.by_service.remove(&job) {
                        if let Some(j) = jobs.list.get_mut(&id) {
                            j.state = if ok { 9 } else { 8 };
                            j.reason = if ok { "job-completed-successfully" } else { "aborted-by-system" };
                        }
                    }
                }
                _ => {}
            }
            // Keep the last 50 jobs.
            if jobs.list.len() > 50 {
                let min = *jobs.list.keys().min().unwrap();
                jobs.list.remove(&min);
            }
        }
    });
}

// ---------- Web page and JSON API ----------

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path).or_else(|| Assets::get("index.html")) {
        Some(f) => {
            let mime = mime_guess::from_path(if Assets::get(path).is_some() { path } else { "index.html" }).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref().to_string())], f.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn api_state(AxState(app): AxState<App>) -> impl IntoResponse {
    Json(app.svc.state())
}

async fn api_settings(AxState(app): AxState<App>) -> impl IntoResponse {
    // Web clients only see what the page needs.
    let mut s = app.svc.settings();
    s.printers.clear();
    s.last_address = None;
    Json(s)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelSummary {
    model_no: String,
    paper_mm: u32,
    width_px: u32,
    dpi: u32,
    gray: bool,
    label: bool,
    a4: bool,
}

async fn api_models() -> impl IntoResponse {
    Json(
        raketo_core::models()
            .iter()
            .map(|m| ModelSummary {
                model_no: m.model_no.clone(),
                paper_mm: m.paper_mm(),
                width_px: m.image_width(),
                dpi: m.devdpi,
                gray: m.gray_print,
                label: m.can_print_label,
                a4: m.is_a4(),
            })
            .collect::<Vec<_>>(),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PagesBody {
    pages: Vec<String>,
    #[serde(default)]
    options: JobOptions,
    model_no: Option<String>,
}

fn decode_pages(pages: &[String]) -> Result<Vec<Vec<u8>>, (StatusCode, String)> {
    pages
        .iter()
        .map(|p| {
            let d = p.split_once(',').map(|(_, d)| d).unwrap_or(p);
            B64.decode(d).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
        })
        .collect()
}

async fn api_preview(AxState(app): AxState<App>, Json(b): Json<PagesBody>) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let pages = decode_pages(&b.pages)?;
    let svc = app.svc.clone();
    let out = tokio::task::spawn_blocking(move || svc.preview(&pages, b.options, b.model_no.as_deref()))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(out.iter().map(|p| format!("data:image/png;base64,{}", B64.encode(p))).collect()))
}

async fn api_print(
    AxState(app): AxState<App>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    Json(b): Json<PagesBody>,
) -> Result<Json<u64>, (StatusCode, String)> {
    let pages = decode_pages(&b.pages)?;
    app.svc
        .print(PrintRequest { pages, options: b.options, source: peer.ip().to_string() })
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn api_events(AxState(app): AxState<App>) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let first = futures::stream::once({
        let st = app.svc.state();
        async move { Ok(SseEvent::default().data(serde_json::to_string(&ServiceEvent::State(st)).unwrap_or_default())) }
    });
    let rest = BroadcastStream::new(app.svc.subscribe())
        .filter_map(|e| e.ok())
        .map(|e| Ok(SseEvent::default().data(serde_json::to_string(&e).unwrap_or_default())));
    Sse::new(first.chain(rest)).keep_alive(KeepAlive::default())
}

// ---------- IPP ----------

fn host_from(headers: &HeaderMap) -> String {
    headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost").to_string()
}

async fn ipp_handler(AxState(app): AxState<App>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(req) = ipp::parse(&body) else {
        return (StatusCode::BAD_REQUEST, "bad ipp").into_response();
    };
    let host = host_from(&headers);
    let resp = match req.code {
        op::GET_PRINTER_ATTRIBUTES => get_printer_attributes(&app, &req, &host),
        op::VALIDATE_JOB => validate(&app, &req).unwrap_or_else(|| Message::response(&req, status::OK)),
        op::PRINT_JOB => print_job(&app, &req, &host, true),
        op::CREATE_JOB => print_job(&app, &req, &host, false),
        op::SEND_DOCUMENT => send_document(&app, &req, &host),
        op::GET_JOB_ATTRIBUTES => get_job(&app, &req, &host),
        op::GET_JOBS => get_jobs(&app, &req, &host),
        op::CANCEL_JOB | op::CLOSE_JOB | op::IDENTIFY_PRINTER => Message::response(&req, status::OK),
        _ => Message::response(&req, status::OPERATION_NOT_SUPPORTED),
    };
    ([(header::CONTENT_TYPE, "application/ipp")], ipp::encode(&resp)).into_response()
}

struct Media {
    name: String,
    w: i32, // hundredths of mm
    h: i32,
}

fn media_list(paper_mm: u32) -> Vec<Media> {
    let w = paper_mm as i32 * 100;
    let mut v: Vec<Media> = [80, 150, 250]
        .iter()
        .map(|h| Media { name: format!("custom_ticket-{paper_mm}x{h}_{paper_mm}x{h}mm"), w, h: h * 100 })
        .collect();
    v.push(Media { name: "iso_a6_105x148mm".into(), w: 10500, h: 14800 });
    v.push(Media { name: "iso_a4_210x297mm".into(), w: 21000, h: 29700 });
    v.push(Media { name: "na_letter_8.5x11in".into(), w: 21590, h: 27940 });
    v
}

fn media_col(m: &Media) -> Value {
    Value::Collection(vec![
        (
            "media-size".into(),
            vec![Value::Collection(vec![("x-dimension".into(), vec![Value::Int(m.w)]), ("y-dimension".into(), vec![Value::Int(m.h)])])],
        ),
        ("media-top-margin".into(), vec![Value::Int(0)]),
        ("media-bottom-margin".into(), vec![Value::Int(0)]),
        ("media-left-margin".into(), vec![Value::Int(0)]),
        ("media-right-margin".into(), vec![Value::Int(0)]),
        ("media-source".into(), vec![Value::kw("main-roll")]),
        ("media-type".into(), vec![Value::kw("stationery")]),
    ])
}

fn printer_state(app: &App) -> (i32, Vec<Value>, bool) {
    let st = app.svc.state();
    match (&st.printer, &st.status) {
        (None, _) => (5, vec![Value::kw("offline-report")], false),
        (Some(_), Some(s)) if s.no_paper => (5, vec![Value::kw("media-empty-error")], true),
        (Some(_), Some(s)) if s.cover_open => (5, vec![Value::kw("cover-open-error")], true),
        (Some(_), _) if st.busy => (4, vec![Value::kw("none")], true),
        _ => (3, vec![Value::kw("none")], true),
    }
}

fn get_printer_attributes(app: &App, req: &Message, host: &str) -> Message {
    let st = app.svc.state();
    let model = st.printer.as_ref().and_then(|p| raketo_core::lookup(&p.model_no));
    let (paper_mm, dpi, model_no) = model.map(|m| (m.paper_mm(), m.devdpi as i32, m.model_no.clone())).unwrap_or((58, 203, "thermal".into()));
    let media = media_list(paper_mm);
    let (state, reasons, accepting) = printer_state(app);
    let uri = format!("ipp://{host}/ipp/print");
    let mut p = Group::new(tag::PRINTER);
    p.add("printer-uri-supported", Value::uri(&uri))
        .add("uri-security-supported", Value::kw("none"))
        .add("uri-authentication-supported", Value::kw("none"))
        .add("printer-name", Value::name(&app.name))
        .add("printer-info", Value::text(&app.name))
        .add("printer-make-and-model", Value::text(&format!("Raketo {model_no}")))
        .add("printer-location", Value::text(""))
        .add("printer-more-info", Value::uri(&format!("http://{host}/")))
        .add("printer-icons", Value::uri(&format!("http://{host}/icon.png")))
        .add("printer-uuid", Value::uri(&format!("urn:uuid:{}", app.uuid)))
        .add("printer-device-id", Value::text(&format!("MFG:Raketo;MDL:{model_no};CMD:PWGRaster,URF,JPEG;")))
        .add("printer-state", Value::Enum(state))
        .add_many("printer-state-reasons", reasons)
        .add("printer-is-accepting-jobs", Value::Bool(accepting))
        .add("printer-up-time", Value::Int(app.started.elapsed().as_secs().max(1) as i32))
        .add("queued-job-count", Value::Int(st.queue as i32 + st.busy as i32))
        .add_many("ipp-versions-supported", vec![Value::kw("1.1"), Value::kw("2.0")])
        .add_many("ipp-features-supported", vec![Value::kw("ipp-everywhere")])
        .add_many(
            "operations-supported",
            [op::PRINT_JOB, op::VALIDATE_JOB, op::CREATE_JOB, op::SEND_DOCUMENT, op::CANCEL_JOB, op::GET_JOB_ATTRIBUTES, op::GET_JOBS, op::GET_PRINTER_ATTRIBUTES, op::CLOSE_JOB, op::IDENTIFY_PRINTER]
                .iter()
                .map(|o| Value::Enum(*o as i32))
                .collect(),
        )
        .add("charset-configured", Value::Str(tag::CHARSET, "utf-8".into()))
        .add("charset-supported", Value::Str(tag::CHARSET, "utf-8".into()))
        .add("natural-language-configured", Value::Str(tag::LANGUAGE, "en".into()))
        .add("generated-natural-language-supported", Value::Str(tag::LANGUAGE, "en".into()))
        .add("document-format-default", Value::mime("image/pwg-raster"))
        .add_many(
            "document-format-supported",
            ["image/pwg-raster", "image/urf", "image/jpeg", "image/png", "application/octet-stream"].iter().map(|m| Value::mime(m)).collect(),
        )
        .add("pdl-override-supported", Value::kw("attempted"))
        .add_many("compression-supported", vec![Value::kw("none")])
        .add("color-supported", Value::Bool(false))
        .add("print-color-mode-default", Value::kw("monochrome"))
        .add_many("print-color-mode-supported", vec![Value::kw("auto"), Value::kw("monochrome")])
        .add("output-mode-default", Value::kw("monochrome"))
        .add_many("output-mode-supported", vec![Value::kw("monochrome")])
        .add("sides-default", Value::kw("one-sided"))
        .add_many("sides-supported", vec![Value::kw("one-sided")])
        .add("copies-default", Value::Int(1))
        .add("copies-supported", Value::Range(1, 50))
        .add("print-quality-default", Value::Enum(4))
        .add_many("print-quality-supported", vec![Value::Enum(3), Value::Enum(4), Value::Enum(5)])
        .add("printer-resolution-default", Value::Resolution(dpi, dpi))
        .add_many("printer-resolution-supported", vec![Value::Resolution(dpi, dpi)])
        .add_many("pwg-raster-document-resolution-supported", vec![Value::Resolution(dpi, dpi)])
        .add_many("pwg-raster-document-type-supported", vec![Value::kw("black_1"), Value::kw("sgray_8")])
        .add("pwg-raster-document-sheet-back", Value::kw("normal"))
        .add_many("urf-supported", ["V1.4", "CP1", "W8", &format!("RS{dpi}")].iter().map(|s| Value::kw(s)).collect())
        .add("media-default", Value::kw(&media[1].name))
        .add_many("media-supported", media.iter().map(|m| Value::kw(&m.name)).collect())
        .add_many("media-ready", media.iter().take(3).map(|m| Value::kw(&m.name)).collect())
        .add("media-col-default", media_col(&media[1]))
        .add_many("media-col-database", media.iter().map(media_col).collect())
        .add_many("media-col-ready", media.iter().take(3).map(media_col).collect())
        .add_many(
            "media-col-supported",
            ["media-size", "media-top-margin", "media-bottom-margin", "media-left-margin", "media-right-margin", "media-source", "media-type"]
                .iter()
                .map(|s| Value::kw(s))
                .collect(),
        )
        .add_many("media-size-supported", media.iter().map(|m| Value::Collection(vec![("x-dimension".into(), vec![Value::Int(m.w)]), ("y-dimension".into(), vec![Value::Int(m.h)])])).collect())
        .add_many("media-source-supported", vec![Value::kw("main-roll")])
        .add_many("media-type-supported", vec![Value::kw("stationery")])
        .add_many("media-top-margin-supported", vec![Value::Int(0)])
        .add_many("media-bottom-margin-supported", vec![Value::Int(0)])
        .add_many("media-left-margin-supported", vec![Value::Int(0)])
        .add_many("media-right-margin-supported", vec![Value::Int(0)])
        .add("orientation-requested-default", Value::Enum(3))
        .add_many("orientation-requested-supported", vec![Value::Enum(3), Value::Enum(4)])
        .add("multiple-document-jobs-supported", Value::Bool(false))
        .add("multiple-operation-time-out", Value::Int(60))
        .add_many("which-jobs-supported", vec![Value::kw("completed"), Value::kw("not-completed")])
        .add_many(
            "job-creation-attributes-supported",
            ["copies", "media", "media-col", "print-quality", "print-color-mode", "orientation-requested", "sides"].iter().map(|s| Value::kw(s)).collect(),
        )
        .add("printer-kind", Value::kw("receipt"));

    // Honour requested-attributes when present.
    let wanted: Option<Vec<String>> = req.group(tag::OPERATION).and_then(|g| {
        g.attrs.iter().find(|(n, _)| n == "requested-attributes").map(|(_, v)| v.iter().filter_map(|x| x.as_str().map(String::from)).collect())
    });
    if let Some(w) = wanted {
        if !w.iter().any(|x| x == "all" || x == "printer-description" || x == "printer-status" || x == "media-col-database") {
            p.attrs.retain(|(n, _)| w.contains(n));
        }
    }
    let mut resp = Message::response(req, status::OK);
    resp.groups.push(p);
    resp
}

fn validate(app: &App, req: &Message) -> Option<Message> {
    let (_, _, accepting) = printer_state(app);
    if !accepting {
        return Some(Message::response(req, status::NOT_ACCEPTING));
    }
    if let Some(f) = req.op_attr("document-format").and_then(|v| v.as_str()) {
        if !["image/pwg-raster", "image/urf", "image/jpeg", "image/png", "application/octet-stream"].contains(&f) {
            return Some(Message::response(req, status::DOCUMENT_FORMAT_NOT_SUPPORTED));
        }
    }
    None
}

fn job_group(j: &IppJob, host: &str) -> Group {
    let mut g = Group::new(tag::JOB);
    g.add("job-id", Value::Int(j.id))
        .add("job-uri", Value::uri(&format!("ipp://{host}/ipp/print/{}", j.id)))
        .add("job-printer-uri", Value::uri(&format!("ipp://{host}/ipp/print")))
        .add("job-name", Value::name(&j.name))
        .add("job-originating-user-name", Value::name(&j.user))
        .add("job-state", Value::Enum(j.state))
        .add("job-state-reasons", Value::kw(j.reason))
        .add("time-at-creation", Value::Int(j.created as i32))
        .add("job-impressions-completed", Value::Int(if j.state == 9 { j.pages } else { 0 }));
    g
}

fn print_job(app: &App, req: &Message, host: &str, with_data: bool) -> Message {
    if let Some(r) = validate(app, req) {
        return r;
    }
    let name = req.op_attr("job-name").and_then(|v| v.as_str()).unwrap_or("Document").to_string();
    let user = req.op_attr("requesting-user-name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let quality = req.job_attr("print-quality").and_then(|v| v.as_int()).unwrap_or(4);
    let copies = req.job_attr("copies").and_then(|v| v.as_int()).unwrap_or(1).clamp(1, 50);
    let job = {
        let mut jobs = app.jobs.lock().unwrap();
        jobs.next += 1;
        let id = jobs.next;
        let j = IppJob { id, name, user, state: 3, reason: "none", created: app.started.elapsed().as_secs().max(1), pages: 0, quality, copies };
        jobs.list.insert(id, j.clone());
        j
    };
    if with_data {
        let format = req.op_attr("document-format").and_then(|v| v.as_str()).unwrap_or("application/octet-stream").to_string();
        if let Err(code) = submit(app, job.id, &format, &req.data) {
            let mut r = Message::response(req, code);
            if let Some(j) = app.jobs.lock().unwrap().list.get_mut(&job.id) {
                j.state = 8;
                j.reason = "document-format-error";
            }
            r.groups.push(job_group(&job, host));
            return r;
        }
    }
    let mut r = Message::response(req, status::OK);
    let j = app.jobs.lock().unwrap().list.get(&job.id).cloned().unwrap_or(job);
    r.groups.push(job_group(&j, host));
    r
}

fn send_document(app: &App, req: &Message, host: &str) -> Message {
    let id = req.op_attr("job-id").and_then(|v| v.as_int()).unwrap_or(0);
    let exists = app.jobs.lock().unwrap().list.contains_key(&id);
    if !exists {
        return Message::response(req, status::NOT_FOUND);
    }
    let format = req.op_attr("document-format").and_then(|v| v.as_str()).unwrap_or("application/octet-stream").to_string();
    if !req.data.is_empty() {
        if let Err(code) = submit(app, id, &format, &req.data) {
            return Message::response(req, code);
        }
    }
    let mut r = Message::response(req, status::OK);
    if let Some(j) = app.jobs.lock().unwrap().list.get(&id) {
        r.groups.push(job_group(j, host));
    }
    r
}

/// Decodes the document, trims blank margins and queues it on the service.
fn submit(app: &App, id: i32, format: &str, data: &[u8]) -> Result<(), u16> {
    let pages = raster::decode(format, data).ok_or(status::DOCUMENT_FORMAT_NOT_SUPPORTED)?;
    let pages: Vec<_> = pages.iter().filter_map(raster::trim).collect();
    let (quality, copies) = {
        let jobs = app.jobs.lock().unwrap();
        let j = jobs.list.get(&id).ok_or(status::NOT_FOUND)?;
        (j.quality, j.copies)
    };
    if pages.is_empty() {
        // Blank document: nothing to print, report success.
        if let Some(j) = app.jobs.lock().unwrap().list.get_mut(&id) {
            j.state = 9;
            j.reason = "job-completed-successfully";
        }
        return Ok(());
    }
    let mut encoded = Vec::new();
    for p in &pages {
        let mut b = Vec::new();
        p.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).map_err(|_| status::DEVICE_ERROR)?;
        encoded.push(b);
    }
    let options = JobOptions {
        // High quality dithers (photos), otherwise crisp text.
        content: if quality >= 5 { Content::Photo } else { Content::Text },
        copies: copies as u32,
        ..Default::default()
    };
    let n = encoded.len() as i32;
    let sid = app
        .svc
        .print(PrintRequest { pages: encoded, options, source: "ipp".into() })
        .map_err(|_| status::DEVICE_ERROR)?;
    let mut jobs = app.jobs.lock().unwrap();
    jobs.by_service.insert(sid, id);
    if let Some(j) = jobs.list.get_mut(&id) {
        j.pages = n;
        j.state = 5;
        j.reason = "job-printing";
    }
    Ok(())
}

fn get_job(app: &App, req: &Message, host: &str) -> Message {
    let id = req.op_attr("job-id").and_then(|v| v.as_int()).unwrap_or(0);
    match app.jobs.lock().unwrap().list.get(&id) {
        Some(j) => {
            let mut r = Message::response(req, status::OK);
            r.groups.push(job_group(j, host));
            r
        }
        None => Message::response(req, status::NOT_FOUND),
    }
}

fn get_jobs(app: &App, req: &Message, host: &str) -> Message {
    let completed = req.op_attr("which-jobs").and_then(|v| v.as_str()) == Some("completed");
    let mut r = Message::response(req, status::OK);
    let jobs = app.jobs.lock().unwrap();
    let mut list: Vec<&IppJob> = jobs.list.values().filter(|j| (j.state >= 7) == completed).collect();
    list.sort_by_key(|j| j.id);
    for j in list {
        r.groups.push(job_group(j, host));
    }
    r
}
