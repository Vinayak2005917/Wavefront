#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod dictation;
mod hotkey;
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
fn delete_transcript(timestamp: u64, text: String, app: tauri::AppHandle) -> Result<(), String> {
    overlay::delete_transcript(&app, timestamp, &text)
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
        .map_err(|e| format!("Could not reload the Zipformer model: {e}"))
}

#[tauri::command]
fn save_dashboard_hotkey(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    shortcut: String,
) -> Result<String, String> {
    let previous = hotkey::saved_shortcut(&app);
    let updated = hotkey::set(&state.hotkey, &shortcut)?;
    if let Err(error) = hotkey::save_shortcut(&app, &updated) {
        let _ = hotkey::set(&state.hotkey, &previous);
        return Err(error);
    }
    Ok(updated)
}

#[tauri::command]
fn toggle_dictation(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let target = unsafe { win::GetForegroundWindow() as isize };
    state
        .commands
        .send(dictation::Command::Toggle {
            target,
            test: false,
        })
        .map_err(|error| format!("Could not toggle dictation: {error}"))
}

#[tauri::command]
fn start_transcription_test(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let target = unsafe { win::GetForegroundWindow() as isize };
    state
        .commands
        .send(dictation::Command::StartTest { target })
        .map_err(|error| format!("Could not start transcription test: {error}"))
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
                "Preparing English Zipformer",
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
            delete_transcript,
            dashboard_recognizer_settings,
            dashboard_hotkey,
            save_recognizer_settings,
            save_dashboard_hotkey,
            toggle_dictation,
            start_transcription_test
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
