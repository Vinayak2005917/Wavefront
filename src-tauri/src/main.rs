#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod dictation;
mod hotkey;
mod nemo;
mod overlay;
mod recognizer;
mod text_injection;
mod transcript;
mod tray;
mod win;

use std::sync::mpsc::Sender;
use tauri::Manager;

pub struct AppState {
    pub commands: Sender<dictation::Command>,
    pub hotkey: hotkey::Controller,
}

#[tauri::command]
fn dashboard_state() -> serde_json::Value {
    overlay::current_state()
}

#[tauri::command]
fn dashboard_history() -> Vec<serde_json::Value> {
    overlay::current_history()
}

#[tauri::command]
fn dashboard_history_recording() -> bool {
    overlay::history_recording_enabled()
}

#[tauri::command]
fn delete_transcript(timestamp: u64, text: String, app: tauri::AppHandle) -> Result<(), String> {
    overlay::delete_transcript(&app, timestamp, &text)
}

#[tauri::command]
fn delete_history(app: tauri::AppHandle) -> Result<(), String> {
    overlay::delete_history(&app)
}

#[tauri::command]
fn set_history_recording(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    overlay::set_history_recording(&app, enabled)
}

#[tauri::command]
fn dashboard_recognizer_settings(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let settings = recognizer::RecognizerSettings::load(&app)?;
    Ok(serde_json::json!({"model_directory": settings.model_directory.to_string_lossy()}))
}

#[tauri::command]
fn dashboard_hotkey(app: tauri::AppHandle) -> String {
    hotkey::saved_shortcut(&app)
}

#[tauri::command]
fn suspend_dashboard_hotkey(state: tauri::State<'_, AppState>) -> Result<(), String> {
    hotkey::suspend(&state.hotkey)
}

#[tauri::command]
fn resume_dashboard_hotkey(state: tauri::State<'_, AppState>) -> Result<(), String> {
    hotkey::resume(&state.hotkey)
}

#[tauri::command]
fn pick_model_directory() -> Option<String> {
    win::pick_directory("Choose a model cache folder")
}

#[tauri::command]
fn save_recognizer_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    model_directory: String,
) -> Result<(), String> {
    let directory = recognizer::normalize_model_directory(&model_directory)?;
    recognizer::RecognizerSettings::save(&app, &directory)?;
    state
        .commands
        .send(dictation::Command::ReloadModel)
        .map_err(|e| format!("Could not reload the local speech model: {e}"))
}

#[tauri::command]
fn save_dashboard_hotkey(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    shortcut: String,
) -> Result<String, String> {
    let previous = hotkey::saved_shortcut(&app);
    let updated = match hotkey::set(&state.hotkey, &shortcut) {
        Ok(updated) => updated,
        Err(error) => {
            let _ = hotkey::suspend(&state.hotkey);
            return Err(error);
        }
    };
    if let Err(error) = hotkey::save_shortcut(&app, &updated) {
        let _ = hotkey::set(&state.hotkey, &previous);
        let _ = hotkey::suspend(&state.hotkey);
        return Err(error);
    }
    if let Err(error) = hotkey::resume(&state.hotkey) {
        let _ = hotkey::suspend(&state.hotkey);
        let _ = hotkey::set(&state.hotkey, &previous);
        let _ = hotkey::save_shortcut(&app, &previous);
        return Err(error);
    }
    Ok(updated)
}

#[tauri::command]
fn toggle_dictation(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let target = unsafe { win::GetForegroundWindow() as isize };
    state
        .commands
        .send(dictation::Command::Toggle { target })
        .map_err(|error| format!("Could not toggle dictation: {error}"))
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            overlay::load_history(&handle);
            // Publish the initial state before starting the worker so its ready/error
            // update cannot be overwritten by a late startup "checking" state.
            overlay::publish(
                &handle,
                "asr_checking",
                "Preparing Nemotron 3.5 ASR",
                "Checking the local model files",
            );
            let commands = dictation::start(handle.clone());
            let hotkey_controller = hotkey::start(
                handle.clone(),
                commands.clone(),
                &hotkey::saved_shortcut(&handle),
            );
            app.manage(AppState {
                commands: commands.clone(),
                hotkey: hotkey_controller,
            });

            if let Some(window) = app.get_webview_window("pill") {
                overlay::prepare(&window);
            }
            tray::install(app)?;
            overlay::show_dashboard(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "dashboard" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            dashboard_state,
            dashboard_history,
            dashboard_history_recording,
            delete_transcript,
            delete_history,
            set_history_recording,
            dashboard_recognizer_settings,
            dashboard_hotkey,
            suspend_dashboard_hotkey,
            resume_dashboard_hotkey,
            pick_model_directory,
            save_recognizer_settings,
            save_dashboard_hotkey,
            toggle_dictation
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Wavefront");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(state) = app_handle.try_state::<AppState>() {
                let _ = state.commands.send(dictation::Command::Shutdown);
            }
            hotkey::stop();
        }
    });
}
