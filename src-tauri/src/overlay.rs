use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::win;

const WINDOW_WIDTH: f64 = 180.0;
const WINDOW_HEIGHT: f64 = 38.0;
const BOTTOM_MARGIN: f64 = 72.0;

#[derive(Clone, serde::Serialize)]
struct UiState<'a> {
    state: &'a str,
    message: &'a str,
    detail: &'a str,
}

static LAST_STATE: OnceLock<Mutex<serde_json::Value>> = OnceLock::new();
static TRANSCRIPT_HISTORY: OnceLock<Mutex<Vec<serde_json::Value>>> = OnceLock::new();
static HISTORY_RECORDING: OnceLock<Mutex<bool>> = OnceLock::new();

fn state_store() -> &'static Mutex<serde_json::Value> {
    LAST_STATE.get_or_init(|| {
        Mutex::new(serde_json::json!({
            "state": "checking",
            "message": "Preparing Nemotron 3.5 ASR",
            "detail": "Checking the local model"
        }))
    })
}

fn transcript_store() -> &'static Mutex<Vec<serde_json::Value>> {
    TRANSCRIPT_HISTORY.get_or_init(|| Mutex::new(Vec::new()))
}

fn history_recording_store() -> &'static Mutex<bool> {
    HISTORY_RECORDING.get_or_init(|| Mutex::new(true))
}

fn transcript_file(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|directory| directory.join("transcriptions.json"))
        .map_err(|error| error.to_string())
}

fn history_recording_file(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|directory| directory.join("record-history.txt"))
        .map_err(|error| error.to_string())
}

fn persist_history(app: &AppHandle, history: &[serde_json::Value]) -> Result<PathBuf, String> {
    let path = transcript_file(app)?;
    fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
        .map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(history).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| format!("Could not save session history: {error}"))?;
    Ok(path)
}

pub fn load_history(app: &AppHandle) {
    if let Ok(path) = history_recording_file(app) {
        let enabled = fs::read_to_string(path)
            .ok()
            .map(|value| value.trim().parse::<bool>().unwrap_or(true))
            .unwrap_or(true);
        if let Ok(mut recording) = history_recording_store().lock() {
            *recording = enabled;
        }
    }

    let Ok(path) = transcript_file(app) else {
        eprintln!("[wavefront] could not locate local transcription history file");
        return;
    };
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) {
            Ok(items) => {
                if let Ok(mut history) = transcript_store().lock() {
                    *history = items;
                }
                eprintln!(
                    "[wavefront] loaded local transcription history from {}",
                    path.display()
                );
            }
            Err(error) => eprintln!("[wavefront] could not parse transcription history: {error}"),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("[wavefront] no saved transcription history yet");
        }
        Err(error) => eprintln!("[wavefront] could not read transcription history: {error}"),
    }
}

pub fn prepare(window: &WebviewWindow) {
    let _ = window.set_focusable(false);
    let _ = window.set_always_on_top(true);
    let _ = window.set_skip_taskbar(true);
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let hwnd = hwnd.0;
            let style = win::GetWindowLongPtrW(hwnd, win::GWL_EXSTYLE);
            win::SetWindowLongPtrW(
                hwnd,
                win::GWL_EXSTYLE,
                style | win::WS_EX_NOACTIVATE | win::WS_EX_TOOLWINDOW,
            );
            win::SetWindowPos(
                hwnd,
                win::HWND_TOPMOST,
                0,
                0,
                0,
                0,
                win::SWP_NOMOVE | win::SWP_NOSIZE | win::SWP_NOACTIVATE | win::SWP_FRAMECHANGED,
            );
            win::ShowWindow(hwnd, win::SW_HIDE);
        }
    }
}

pub fn publish(app: &AppHandle, state: &str, message: &str, detail: &str) {
    let payload = serde_json::to_value(UiState {
        state,
        message,
        detail,
    })
    .unwrap_or_default();
    if let Ok(mut current) = state_store().lock() {
        *current = payload.clone();
    }
    let _ = app.emit("dictation-state", payload);
}

pub fn publish_transcript(app: &AppHandle, text: &str) {
    let Ok(recording) = history_recording_store().lock() else {
        eprintln!("[wavefront] could not read history recording setting");
        return;
    };
    if !*recording {
        return;
    }
    let item = serde_json::json!({
        "text": text,
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
    });
    let (history, save_result) = match transcript_store().lock() {
        Ok(mut history) => {
            history.push(item.clone());
            let save_result = persist_history(app, &history);
            (history.clone(), save_result)
        }
        Err(error) => {
            eprintln!("[wavefront] could not update in-memory transcription history: {error}");
            return;
        }
    };
    drop(recording);
    match save_result {
        Ok(path) => eprintln!("[wavefront] saved transcription to {}", path.display()),
        Err(error) => eprintln!("[wavefront] could not save transcription history: {error}"),
    }
    // Publish the saved snapshot as well as the new item so the dashboard can
    // reconcile against disk-backed history after each completed phrase.
    if let Err(error) = app.emit("transcription-history", history) {
        eprintln!("[wavefront] could not update dashboard transcription history: {error}");
    }
    if let Err(error) = app.emit("transcription", item) {
        eprintln!("[wavefront] could not send transcription to dashboard: {error}");
    }
}

pub fn publish_waveform(app: &AppHandle, samples: &[f32]) {
    let _ = app.emit(
        "dictation-waveform",
        serde_json::json!({ "samples": samples }),
    );
}

pub fn current_state() -> serde_json::Value {
    state_store()
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default()
}

pub fn current_history() -> Vec<serde_json::Value> {
    transcript_store()
        .lock()
        .map(|items| items.clone())
        .unwrap_or_default()
}

pub fn history_recording_enabled() -> bool {
    history_recording_store()
        .lock()
        .map(|recording| *recording)
        .unwrap_or(true)
}

pub fn set_history_recording(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mut recording = history_recording_store()
        .lock()
        .map_err(|error| format!("Could not update history setting: {error}"))?;
    let path = history_recording_file(app)?;
    fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
        .map_err(|error| error.to_string())?;
    let value = if enabled { "true" } else { "false" };
    fs::write(path, value.as_bytes())
        .map_err(|error| format!("Could not save history setting: {error}"))?;
    *recording = enabled;
    Ok(())
}

pub fn delete_history(app: &AppHandle) -> Result<(), String> {
    let mut history = transcript_store()
        .lock()
        .map_err(|error| format!("Could not update session history: {error}"))?;
    let previous = std::mem::take(&mut *history);
    if let Err(error) = persist_history(app, &history) {
        *history = previous;
        return Err(error);
    }
    drop(history);
    if let Err(error) = app.emit("transcription-history", Vec::<serde_json::Value>::new()) {
        eprintln!("[wavefront] could not update dashboard after deleting all history: {error}");
    }
    Ok(())
}

pub fn delete_transcript(app: &AppHandle, timestamp: u64, text: &str) -> Result<(), String> {
    let mut history = transcript_store()
        .lock()
        .map_err(|error| format!("Could not update session history: {error}"))?;
    let index = history
        .iter()
        .position(|item| {
            item.get("timestamp").and_then(serde_json::Value::as_u64) == Some(timestamp)
                && item.get("text").and_then(serde_json::Value::as_str) == Some(text)
        })
        .ok_or_else(|| "This session item no longer exists.".to_owned())?;
    let removed = history.remove(index);
    if let Err(error) = persist_history(app, &history) {
        history.insert(index, removed);
        return Err(error);
    }
    let snapshot = history.clone();
    drop(history);
    if let Err(error) = app.emit("transcription-history", snapshot) {
        eprintln!("[wavefront] could not update dashboard after deleting history item: {error}");
    }
    Ok(())
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("pill") {
        if let Ok(Some(monitor)) = window.primary_monitor() {
            let scale = monitor.scale_factor();
            let size = monitor.size();
            let origin = monitor.position();
            let width = (WINDOW_WIDTH * scale) as i32;
            let height = (WINDOW_HEIGHT * scale) as i32;
            let margin = (BOTTOM_MARGIN * scale) as i32;
            let x = origin.x + ((size.width as i32 - width) / 2);
            let y = origin.y + size.height as i32 - height - margin;
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                win::SetWindowPos(
                    hwnd.0,
                    win::HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    win::SWP_NOMOVE | win::SWP_NOSIZE | win::SWP_NOACTIVATE,
                );
                win::ShowWindow(hwnd.0, win::SW_SHOWNOACTIVATE);
            }
        } else {
            let _ = window.show();
        }
        let _ = app.emit("dictation-show", ());
        let _ = app.emit("dictation-state", current_state());
    }
}

pub fn show_dashboard(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("dashboard") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to("dashboard", "dictation-state", current_state());
        let _ = app.emit_to("dashboard", "transcription-history", current_history());
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("pill") {
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                win::ShowWindow(hwnd.0, win::SW_HIDE);
            }
        } else {
            let _ = window.hide();
        }
    }
}
