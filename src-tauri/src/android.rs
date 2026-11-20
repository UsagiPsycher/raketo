//! Android glue: the Kotlin plugin (`RaketoPlugin`) does Bluetooth and file saving.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, OnceLock};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::Wry;

static HANDLE: OnceLock<PluginHandle<Wry>> = OnceLock::new();

pub fn plugin() -> TauriPlugin<Wry> {
    Builder::new("raketo-android")
        .setup(|_app, api| {
            let h = api.register_android_plugin("app.raketo", "RaketoPlugin")?;
            HANDLE.set(h).ok();
            raketo_bt::android::set_bridge(Arc::new(Bt));
            Ok(())
        })
        .build()
}

fn call<T: serde::de::DeserializeOwned>(cmd: &str, args: serde_json::Value) -> Result<T, String> {
    let h = HANDLE.get().ok_or("Android plugin not ready")?;
    h.run_mobile_plugin(cmd, args).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct Seen {
    name: String,
    address: String,
    rssi: Option<i16>,
}

#[derive(Deserialize)]
struct Devices {
    devices: Vec<Seen>,
}

#[derive(Deserialize)]
struct Opened {
    chunk: usize,
}

struct Bt;

impl raketo_bt::android::Bridge for Bt {
    fn scan(&self, seconds: u32) -> Result<Vec<(String, String, Option<i16>)>, String> {
        let d: Devices = call("scan", json!({ "seconds": seconds }))?;
        Ok(d.devices.into_iter().map(|s| (s.name, s.address, s.rssi)).collect())
    }

    fn open(&self, address: &str, spp: bool, mtu: u16) -> Result<usize, String> {
        let o: Opened = call("open", json!({ "address": address, "spp": spp, "mtu": mtu }))?;
        Ok(o.chunk)
    }

    fn write(&self, data: &[u8]) -> Result<(), String> {
        call::<serde_json::Value>("write", json!({ "data": B64.encode(data) })).map(|_| ())
    }

    fn close(&self) {
        call::<serde_json::Value>("close", json!({})).ok();
    }
}

#[derive(Deserialize)]
struct Saved {
    name: Option<String>,
}

/// Asks where to save a file (system document picker) and writes it; `None` when cancelled.
pub fn save_document(name: &str, contents: &str) -> Result<Option<String>, String> {
    let s: Saved = call("saveDocument", json!({ "name": name, "contents": contents }))?;
    Ok(s.name)
}

// Called by the Kotlin side with bytes from the printer.
#[no_mangle]
pub extern "system" fn Java_app_raketo_Native_onData(env: jni::JNIEnv, _class: jni::objects::JClass, data: jni::objects::JByteArray) {
    if let Ok(bytes) = env.convert_byte_array(&data) {
        raketo_bt::android::on_data(bytes);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_raketo_Native_onClosed(_env: jni::JNIEnv, _class: jni::objects::JClass) {
    raketo_bt::android::on_closed();
}
