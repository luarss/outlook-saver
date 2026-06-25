mod auth;
mod config;
mod saver;
mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use config::AppConfig;

/// Running watcher thread + its stop flag.
struct WatcherHandle {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct WatcherState(Mutex<Option<WatcherHandle>>);

impl WatcherState {
    /// Signals the watcher to stop and waits for the thread to exit.
    fn stop(&self) {
        let taken = self.0.lock().unwrap().take();
        if let Some(WatcherHandle { stop, handle }) = taken {
            stop.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
    }
}

/// Snapshot of app state sent to the frontend.
#[derive(Serialize)]
struct StatusDto {
    client_id: String,
    tenant: String,
    email: Option<String>,
    default_save_dir: Option<String>,
    ask_each_time: bool,
    signed_in: bool,
    watching: bool,
}

fn build_status(state: &WatcherState) -> Result<StatusDto, String> {
    let cfg = AppConfig::load().map_err(|e| format!("{e:#}"))?;
    let signed_in = config::load_refresh_token(&cfg)
        .map(|t| t.is_some())
        .unwrap_or(false);
    let watching = state.0.lock().unwrap().is_some();
    Ok(StatusDto {
        client_id: cfg.client_id,
        tenant: cfg.tenant,
        email: cfg.email,
        default_save_dir: cfg.default_save_dir,
        ask_each_time: cfg.ask_each_time,
        signed_in,
        watching,
    })
}

#[tauri::command]
fn get_status(state: State<WatcherState>) -> Result<StatusDto, String> {
    build_status(&state)
}

#[tauri::command]
fn save_settings(
    client_id: String,
    tenant: String,
    default_save_dir: Option<String>,
    ask_each_time: bool,
) -> Result<(), String> {
    let mut cfg = AppConfig::load().map_err(|e| format!("{e:#}"))?;
    cfg.client_id = client_id.trim().to_string();
    cfg.tenant = if tenant.trim().is_empty() {
        "common".to_string()
    } else {
        tenant.trim().to_string()
    };
    cfg.default_save_dir = default_save_dir.filter(|s| !s.trim().is_empty());
    cfg.ask_each_time = ask_each_time;
    cfg.save().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn login(app: AppHandle) -> Result<StatusDto, String> {
    let app_for_task = app.clone();
    // The login flow blocks (opens a browser, waits for the loopback redirect).
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut cfg = AppConfig::load().map_err(|e| format!("{e:#}"))?;
        let app_for_open = app_for_task.clone();
        auth::interactive_login(&mut cfg, move |url| {
            app_for_open
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|e| anyhow::anyhow!("could not open browser: {e}"))
        })
        .map_err(|e| format!("{e:#}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("login task failed: {e}"))??;

    build_status(&app.state::<WatcherState>())
}

#[tauri::command]
fn logout(state: State<WatcherState>) -> Result<(), String> {
    state.stop();
    let cfg = AppConfig::load().map_err(|e| format!("{e:#}"))?;
    config::delete_refresh_token(&cfg).map_err(|e| format!("{e:#}"))?;
    let mut cleared = cfg;
    cleared.email = None;
    cleared.save().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn start_watching(app: AppHandle, state: State<WatcherState>) -> Result<(), String> {
    watcher::preflight().map_err(|e| format!("{e:#}"))?;

    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Ok(()); // already running
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let app_for_thread = app.clone();
    let handle = std::thread::spawn(move || {
        watcher::run(app_for_thread, stop_for_thread);
    });
    *guard = Some(WatcherHandle { stop, handle });
    Ok(())
}

#[tauri::command]
fn stop_watching(state: State<WatcherState>) -> Result<(), String> {
    state.stop();
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(WatcherState::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            save_settings,
            login,
            logout,
            start_watching,
            stop_watching
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
