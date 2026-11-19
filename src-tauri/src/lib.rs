use base64::{engine::general_purpose::STANDARD as B64, Engine};
use raketo_core::JobOptions;
use raketo_service::{PrintRequest, PrinterSettings, Seen, Service, Settings, State};
use serde::Serialize;
use raketo_server::{Server, ShareInfo};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

#[cfg(windows)]
mod firewall;

/// The running server with its mode (`true` = shared on the LAN), and the
/// firewall check done this session.
#[derive(Default)]
struct Share(Mutex<Option<(Server, bool)>>, Mutex<Option<bool>>);

const DEFAULT_PORT: u16 = 8631;

fn share_name(svc: &Service) -> String {
    let s = svc.settings();
    s.lan_name.clone().unwrap_or_else(|| {
        let printer = svc.state().printer.map(|p| p.name).unwrap_or_else(|| "Raketo".into());
        format!("{printer} (Raketo)")
    })
}

/// Tells local clients (the `raketo` CLI) where the API listens.
fn write_discovery(app: &tauri::AppHandle, port: u16) {
    if let Ok(dir) = app.path().app_config_dir() {
        std::fs::create_dir_all(&dir).ok();
        let body = serde_json::json!({ "port": port, "url": format!("http://127.0.0.1:{port}"), "pid": std::process::id() });
        std::fs::write(dir.join("api.json"), body.to_string()).ok();
    }
}

async fn firewall_allowed(share: &Share) -> bool {
    let mut checked = share.1.lock().await;
    if let Some(v) = *checked {
        return v;
    }
    #[cfg(windows)]
    let v = tokio::task::spawn_blocking(firewall::ensure).await.unwrap_or(false);
    #[cfg(not(windows))]
    let v = true;
    *checked = Some(v);
    v
}

/// Keeps the server in the right mode: shared on the LAN while sharing is on
/// and a printer is connected, local-only (API for this computer) otherwise.
async fn sync_share(app: &tauri::AppHandle) {
    let svc = app.state::<Service>().inner().clone();
    let share = app.state::<Share>();
    let mut cur = share.0.lock().await;
    let lan = svc.settings().share_on_lan && svc.state().printer.is_some();
    if cur.as_ref().is_some_and(|(_, mode)| *mode == lan) {
        return;
    }
    if let Some((s, _)) = cur.take() {
        s.stop();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    // Open the firewall before the port, so Windows never prompts.
    let allowed = if lan { firewall_allowed(&share).await } else { true };
    let configured = svc.settings().lan_port.unwrap_or(DEFAULT_PORT);
    // Port 80 gives a plain http://raketo.local address when it is free.
    let mut ports = if lan { vec![80, configured, 0] } else { vec![configured, 0] };
    ports.dedup();
    for port in ports {
        if let Ok(mut s) = raketo_server::start(svc.clone(), port, share_name(&svc), lan).await {
            s.info.firewall_blocked = !allowed;
            write_discovery(app, s.info.port);
            app.emit("share", if lan { Some(s.info.clone()) } else { None }).ok();
            *cur = Some((s, lan));
            break;
        }
    }
}

#[tauri::command]
async fn share_info(share: tauri::State<'_, Share>) -> Res<Option<ShareInfo>> {
    Ok(share.0.lock().await.as_ref().filter(|(_, lan)| *lan).map(|(s, _)| s.info.clone()))
}

#[tauri::command]
async fn set_share(app: tauri::AppHandle, svc: tauri::State<'_, Service>, enabled: bool) -> Res<Settings> {
    let s = svc.update_settings(|s| s.share_on_lan = enabled);
    sync_share(&app).await;
    Ok(s)
}

/// Retries the firewall setup (after the user declined the prompt).
#[tauri::command]
async fn retry_firewall(app: tauri::AppHandle) -> Res<()> {
    let share = app.state::<Share>();
    *share.1.lock().await = None;
    if let Some((s, _)) = share.0.lock().await.take() {
        s.stop();
    }
    sync_share(&app).await;
    Ok(())
}

type Res<T> = Result<T, String>;

fn err(e: impl ToString) -> String {
    e.to_string()
}

fn decode_pages(pages: &[String]) -> Res<Vec<Vec<u8>>> {
    pages
        .iter()
        .map(|p| {
            let data = p.split_once(',').map(|(_, d)| d).unwrap_or(p);
            B64.decode(data).map_err(err)
        })
        .collect()
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

#[tauri::command]
fn models() -> Vec<ModelSummary> {
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
        .collect()
}

#[tauri::command]
async fn scan(svc: tauri::State<'_, Service>, seconds: Option<u64>) -> Res<Vec<Seen>> {
    svc.scan(seconds.unwrap_or(6)).await.map_err(err)
}

#[tauri::command]
async fn connect(svc: tauri::State<'_, Service>, address: String, name: Option<String>) -> Res<raketo_service::State> {
    svc.connect(&address, name.as_deref()).await.map_err(err)?;
    Ok(svc.state())
}

#[tauri::command]
async fn reconnect(svc: tauri::State<'_, Service>) -> Res<State> {
    svc.reconnect_last().await.map_err(err)?;
    Ok(svc.state())
}

#[tauri::command]
async fn disconnect(svc: tauri::State<'_, Service>) -> Res<State> {
    svc.disconnect().await;
    Ok(svc.state())
}

#[tauri::command]
fn state(svc: tauri::State<'_, Service>) -> State {
    svc.state()
}

#[tauri::command]
fn settings(svc: tauri::State<'_, Service>) -> Settings {
    svc.settings()
}

#[tauri::command]
fn set_printer_settings(svc: tauri::State<'_, Service>, address: String, value: PrinterSettings) -> Settings {
    svc.update_settings(|s| {
        s.printers.insert(address, value);
    })
}

#[tauri::command]
async fn preview(
    svc: tauri::State<'_, Service>,
    pages: Vec<String>,
    options: JobOptions,
    model_no: Option<String>,
) -> Res<Vec<String>> {
    let pages = decode_pages(&pages)?;
    let svc = svc.inner().clone();
    tokio::task::spawn_blocking(move || svc.preview(&pages, options, model_no.as_deref()))
        .await
        .map_err(err)?
        .map(|v| v.iter().map(|p| format!("data:image/png;base64,{}", B64.encode(p))).collect())
        .map_err(err)
}

#[tauri::command]
async fn print(svc: tauri::State<'_, Service>, pages: Vec<String>, options: JobOptions) -> Res<u64> {
    let pages = decode_pages(&pages)?;
    svc.print(PrintRequest { pages, options, source: "local".into() }).map_err(err)
}

#[tauri::command]
async fn print_gray_wedge(svc: tauri::State<'_, Service>) -> Res<u64> {
    svc.print_gray_wedge().map_err(err)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir().ok();
            let svc = Service::new(dir);
            let handle = app.handle().clone();
            let (s2, mut rx) = (svc.clone(), svc.subscribe());
            let h0 = handle.clone();
            tauri::async_runtime::spawn(async move {
                s2.start();
                // Local API from launch, even before a printer connects.
                sync_share(&h0).await;
                let mut connected = false;
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            if let raketo_service::ServiceEvent::State(st) = &ev {
                                if st.printer.is_some() != connected {
                                    connected = st.printer.is_some();
                                    let h = handle.clone();
                                    tauri::async_runtime::spawn(async move { sync_share(&h).await });
                                }
                            }
                            handle.emit("service", ev).ok();
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            app.manage(svc);
            app.manage(Share::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            models,
            scan,
            connect,
            reconnect,
            disconnect,
            state,
            settings,
            set_printer_settings,
            preview,
            print,
            print_gray_wedge,
            share_info,
            set_share,
            retry_firewall
        ])
        .run(tauri::generate_context!())
        .expect("error while running Raketo");
}
