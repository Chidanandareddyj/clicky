//! Clicky-style companion: tray, push-to-talk, capture, Deepgram listen, playback.

mod audio;
mod capture;
mod deepgram;
mod tts;

use audio::MicCaptureStreaming;
use capture::{capture_all_screens, enumerate_monitors, CapturedScreen, MonitorDescription};
use deepgram::run_deepgram_listen_session;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, Manager, PhysicalPosition, PhysicalSize,
};
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};
use tokio::sync::{mpsc::unbounded_channel, watch};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Settings {
    proxy_base_url: String,
    openrouter_model: String,
    deepgram_tts_model: String,
    #[serde(default)]
    show_buddy_always: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            proxy_base_url: String::new(),
            openrouter_model: "openai/gpt-4o-mini".to_string(),
            deepgram_tts_model: "aura-2-thalia-en".to_string(),
            show_buddy_always: true,
        }
    }
}

struct SettingsStore {
    path: PathBuf,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    fn load(app_handle: &AppHandle) -> Result<Self, String> {
        let dir = app_handle
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("clicky-settings.json");
        let inner = if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            Settings::default()
        };
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
    }

    fn snapshot(&self) -> Settings {
        self.inner.lock().expect("settings mutex").clone()
    }

    fn persist(&self, next: Settings) -> Result<(), String> {
        *self
            .inner
            .lock()
            .map_err(|_| String::from("settings mutex poisoned"))? = next.clone();
        let raw = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
        fs::write(&self.path, raw).map_err(|e| e.to_string())
    }
}

struct ListenCtl {
    worker: Mutex<Option<JoinHandle<Result<(), String>>>>,
    pcm_should_continue: Mutex<Option<Arc<AtomicBool>>>,
}

impl ListenCtl {
    fn new() -> Self {
        Self {
            worker: Mutex::new(None),
            pcm_should_continue: Mutex::new(None),
        }
    }

    fn join_worker(&self) {
        if let Ok(mut guard) = self.worker.lock() {
            if let Some(previous) = guard.take() {
                let _ = previous.join();
            }
        }
    }

    fn set_pcm_continue(&self, flag: Arc<AtomicBool>) {
        if let Ok(mut guard) = self.pcm_should_continue.lock() {
            *guard = Some(flag);
        }
    }

    fn stop_pcm(&self) {
        if let Ok(mut guard) = self.pcm_should_continue.lock() {
            if let Some(flag) = guard.take() {
                flag.store(false, Ordering::Release);
            }
        }
        std::thread::sleep(Duration::from_millis(60));
    }
}

fn emit_virtual_bounds(app: &AppHandle) {
    if let Ok(monitors) = enumerate_monitors() {
        let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
        let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
        let max_right = monitors
            .iter()
            .map(|m| m.x + m.width as i32)
            .max()
            .unwrap_or(0);
        let max_bottom = monitors
            .iter()
            .map(|m| m.y + m.height as i32)
            .max()
            .unwrap_or(0);
        let width_px = max_right.saturating_sub(min_x).max(1) as u32;
        let height_px = max_bottom.saturating_sub(min_y).max(1) as u32;

        let _ = app.emit(
            "virtual-desktop-bounds",
            serde_json::json!({
              "origin": { "x": min_x, "y": min_y },
              "width": width_px,
              "height": height_px,
              "monitors": monitors,
            }),
        );

        if let Some(win) = app.get_webview_window("overlay") {
            let _ = win.set_position(PhysicalPosition::new(min_x, min_y));
            let _ = win.set_size(PhysicalSize::new(width_px, height_px));
        }
    }
}

#[tauri::command]
fn companion_load_settings(store: tauri::State<'_, Arc<SettingsStore>>) -> Settings {
    store.snapshot()
}

#[tauri::command]
fn companion_save_settings(
    payload: Settings,
    store: tauri::State<'_, Arc<SettingsStore>>,
) -> Result<(), String> {
    store.persist(payload)
}

#[tauri::command]
fn companion_set_proxy_url(
    proxy_base_url: String,
    store: tauri::State<'_, Arc<SettingsStore>>,
) -> Result<(), String> {
    let mut merged = store.snapshot();
    merged.proxy_base_url = proxy_base_url.trim_end_matches('/').to_string();
    store.persist(merged)
}

#[tauri::command]
fn companion_enumerate_monitors_command() -> Result<Vec<MonitorDescription>, String> {
    enumerate_monitors().map_err(|e| e.to_string())
}

#[tauri::command]
fn companion_capture_all_screens_command() -> Result<Vec<CapturedScreen>, String> {
    capture_all_screens().map_err(|e| e.to_string())
}

#[tauri::command]
fn companion_overlay_set_click_through(
    ignore_cursor_events: bool,
    app_handle: AppHandle,
) -> Result<(), String> {
    if let Some(win) = app_handle.get_webview_window("overlay") {
        win.set_ignore_cursor_events(ignore_cursor_events)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn companion_overlay_show(show: bool, app_handle: AppHandle) -> Result<(), String> {
    if let Some(win) = app_handle.get_webview_window("overlay") {
        if show {
            win.show().map_err(|e| e.to_string())?;
            emit_virtual_bounds(&app_handle);
        } else {
            win.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn companion_play_tts(
    text_plain: String,
    app_handle: AppHandle,
    store: tauri::State<'_, Arc<SettingsStore>>,
) -> Result<(), String> {
    let settings_snapshot = store.snapshot();
    if settings_snapshot.proxy_base_url.trim().is_empty() {
        return Err("Configure proxy Worker base URL".into());
    }
    let base = settings_snapshot.proxy_base_url.trim_end_matches('/');
    let resp = reqwest::Client::new()
        .post(format!("{base}/tts"))
        .json(&serde_json::json!({
            "text": text_plain,
            "model": settings_snapshot.deepgram_tts_model,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(resp.text().await.unwrap_or_else(|_| "tts failed".into()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    tts::enqueue_play_deepgram_mp3_bytes(bytes.to_vec())?;
    let _ = app_handle.emit("tts-state", serde_json::json!({ "playing": true }));
    Ok(())
}

fn toggle_panel(app: &AppHandle) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };
    match panel.is_visible() {
        Ok(true) => {
            panel.hide().ok();
        }
        _ => {
            panel.set_position(LogicalPosition::new(96.0, 96.0)).ok();
            panel.show().ok();
            panel.set_focus().ok();
        }
    }
}

fn tray_menu_event(app_handle: &AppHandle, menu_id: &str) {
    match menu_id {
        "toggle_panel" => toggle_panel(app_handle),
        "quit" => app_handle.exit(0),
        _ => {}
    }
}

fn begin_listen(app: &AppHandle) {
    let listen_ctl_measurement_arc_inside: Arc<ListenCtl> =
        app.state::<Arc<ListenCtl>>().inner().clone();
    listen_ctl_measurement_arc_inside.join_worker();
    listen_ctl_measurement_arc_inside.stop_pcm();

    let settings_measurement_snapshot_inside = (*app.state::<Arc<SettingsStore>>()).snapshot();
    if settings_measurement_snapshot_inside
        .proxy_base_url
        .trim()
        .is_empty()
    {
        let _ = app.emit(
            "listen-error",
            serde_json::json!({ "message": "Configure Cloudflare Worker proxy URL" }),
        );
        return;
    }

    let pcm_continue_local = Arc::new(AtomicBool::new(true));
    listen_ctl_measurement_arc_inside.set_pcm_continue(Arc::clone(&pcm_continue_local));

    let (pcm_std_tx, pcm_std_rx) = std::sync::mpsc::channel::<Vec<i16>>();
    let (tok_tx, tok_rx) = unbounded_channel::<Vec<i16>>();

    let pcm_flag_for_mic = Arc::clone(&pcm_continue_local);
    let pcm_spin_flag_measurement = Arc::clone(&pcm_continue_local);
    let pcm_std_tx_for_mic = pcm_std_tx.clone();

    let microphone_thread = std::thread::spawn(move || {
        let mic_result = MicCaptureStreaming::start(pcm_std_tx_for_mic, 16_000, pcm_flag_for_mic);
        if let Err(e) = mic_result {
            eprintln!("mic start failed: {e}");
            return;
        }
        while pcm_spin_flag_measurement.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(25));
        }
    });

    let pcm_flag_for_bridge_forwarder = Arc::clone(&pcm_continue_local);
    std::thread::spawn(move || {
        loop {
            match pcm_std_rx.recv_timeout(Duration::from_millis(120)) {
                Ok(chunk) => {
                    if tok_tx.send(chunk).is_err() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if !pcm_flag_for_bridge_forwarder.load(Ordering::Acquire) {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !pcm_flag_for_bridge_forwarder.load(Ordering::Acquire) {
                break;
            }
        }
        drop(tok_tx);
    });

    let app_clone = app.clone();
    let store_clone: Arc<SettingsStore> = app.state::<Arc<SettingsStore>>().inner().clone();
    let proxy_move = settings_measurement_snapshot_inside.proxy_base_url.clone();

    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        let (_cancel_unused_tx, cancel_rx) = watch::channel(false);
        let deepgram_finish = runtime.block_on(run_deepgram_listen_session(
            app_clone.clone(),
            proxy_move.trim().to_owned(),
            tok_rx,
            cancel_rx,
        ));

        let _ = microphone_thread.join();

        match deepgram_finish {
            Ok(transcript) => {
                let _ = app_clone.emit(
                    "listen-done",
                    serde_json::json!({ "transcript": transcript }),
                );
                let buddy_visible =
                    store_clone.snapshot().show_buddy_always || !transcript.is_empty();
                if buddy_visible {
                    if let Some(overlay_win) = app_clone.get_webview_window("overlay") {
                        overlay_win.show().ok();
                        emit_virtual_bounds(&app_clone);
                    }
                }
            }
            Err(err) => {
                let _ = app_clone.emit("listen-error", serde_json::json!({ "message": err }));
            }
        }

        let _ = app_clone.emit("buddy-listen-state", serde_json::json!({ "state": "idle" }));

        Ok(())
    });

    let _ = app.emit(
        "buddy-listen-state",
        serde_json::json!({ "state": "listening" }),
    );

    if let Ok(mut guard) = app.state::<Arc<ListenCtl>>().worker.lock() {
        *guard = Some(worker);
    }
}

fn end_listen(app: &AppHandle) {
    app.state::<Arc<ListenCtl>>().stop_pcm();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            ShortcutBuilder::new()
                .with_shortcuts(["Ctrl+Alt+Space"])
                .expect("register shortcut")
                .with_handler(|shortcut_app_handle, _, ev| match ev.state() {
                    ShortcutState::Pressed => begin_listen(shortcut_app_handle),
                    ShortcutState::Released => end_listen(shortcut_app_handle),
                })
                .build(),
        )
        .setup(move |app| {
            app.manage(Arc::new(SettingsStore::load(app.handle())?));
            app.manage(Arc::new(ListenCtl::new()));

            let tray_menu = Menu::with_items(
                app.handle(),
                &[
                    &MenuItem::with_id(
                        app.handle(),
                        "toggle_panel",
                        "Toggle panel",
                        true,
                        None::<&str>,
                    )?,
                    &MenuItem::with_id(app.handle(), "quit", "Quit", true, None::<&str>)?,
                ],
            )?;

            let mut tray = TrayIconBuilder::with_id("clicky_tray")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|tauri_app_menu, evt| {
                    tray_menu_event(tauri_app_menu, evt.id.as_ref())
                })
                .on_tray_icon_event(|tray, evt| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = evt
                    {
                        toggle_panel(tray.app_handle());
                    }
                });

            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }

            tray.build(app.handle())?;

            emit_virtual_bounds(app.handle());

            let show_overlay = (*app.state::<Arc<SettingsStore>>())
                .snapshot()
                .show_buddy_always;
            if show_overlay {
                if let Some(overlay) = app.get_webview_window("overlay") {
                    overlay.show().ok();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            companion_load_settings,
            companion_save_settings,
            companion_set_proxy_url,
            companion_enumerate_monitors_command,
            companion_capture_all_screens_command,
            companion_overlay_show,
            companion_overlay_set_click_through,
            companion_play_tts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
