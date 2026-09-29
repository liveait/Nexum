// Prevents additional console window on Windows in debug mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop_lib;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DesktopSettings {
    server: String,
    refresh_interval_secs: u64,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            server: "127.0.0.1:39100".to_owned(),
            refresh_interval_secs: 5,
        }
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("could not resolve application settings directory: {error}"))
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Result<DesktopSettings, String> {
    let path = settings_path(&app)?;
    if !path.is_file() {
        return Ok(DesktopSettings::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read settings {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("could not parse settings {}: {error}", path.display()))
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: DesktopSettings) -> Result<(), String> {
    let path = settings_path(&app)?;
    let directory = path
        .parent()
        .ok_or_else(|| "application settings path has no parent".to_owned())?;
    std::fs::create_dir_all(directory).map_err(|error| {
        format!(
            "could not create application settings directory {}: {error}",
            directory.display()
        )
    })?;
    let content = serde_json::to_string_pretty(&settings)
        .map_err(|error| format!("could not serialize settings: {error}"))?;
    std::fs::write(&path, content)
        .map_err(|error| format!("could not write settings {}: {error}", path.display()))
}

#[tauri::command]
fn credential_status(server: String) -> Result<Option<String>, String> {
    desktop_lib::credential_status(&server)
}

#[tauri::command]
fn save_credential(server: String, scheme: String, secret: String) -> Result<(), String> {
    desktop_lib::save_credential(&server, &scheme, secret)
}

#[tauri::command]
fn clear_credential(server: String) -> Result<(), String> {
    desktop_lib::clear_credential(&server)
}

#[tauri::command]
fn server_version(server: String) -> Result<String, String> {
    desktop_lib::server_version(&server, 5000)
}

#[tauri::command]
fn task_list(server: String) -> Result<serde_json::Value, String> {
    let tasks = desktop_lib::task_list(&server, 5000)?;
    serde_json::to_value(tasks).map_err(|e| e.to_string())
}

#[tauri::command]
fn task_get(server: String, task_id: String) -> Result<serde_json::Value, String> {
    desktop_lib::task_get(&server, &task_id, 5000)
}

#[tauri::command]
fn task_create(
    server: String,
    id: String,
    source: String,
    destination: String,
) -> Result<serde_json::Value, String> {
    desktop_lib::task_create(&server, &id, &source, &destination, 30000)
}

#[tauri::command]
fn task_queue(server: String, task_id: String) -> Result<bool, String> {
    desktop_lib::task_queue(&server, &task_id, 5000)
}

#[tauri::command]
fn task_start(server: String) -> Result<String, String> {
    desktop_lib::task_start(&server, 5000)
}

#[tauri::command]
fn task_pause(server: String, task_id: String) -> Result<bool, String> {
    desktop_lib::task_pause(&server, &task_id, 5000)
}

#[tauri::command]
fn task_resume(server: String, task_id: String) -> Result<bool, String> {
    desktop_lib::task_resume(&server, &task_id, 5000)
}

#[tauri::command]
fn task_remove(server: String, task_id: String) -> Result<bool, String> {
    desktop_lib::task_remove(&server, &task_id, 5000)
}

#[tauri::command]
fn start_event_stream(
    app: AppHandle,
    state: tauri::State<'_, desktop_lib::EventSubscriptionManager>,
    server: String,
) -> Result<u64, String> {
    if server.trim().is_empty() {
        return Err("server address must not be empty".to_owned());
    }
    Ok(state.start(app, server))
}

#[tauri::command]
fn stop_event_stream(
    state: tauri::State<'_, desktop_lib::EventSubscriptionManager>,
    generation: u64,
) -> Result<(), String> {
    state.stop(generation);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(desktop_lib::EventSubscriptionManager::default())
        .invoke_handler(tauri::generate_handler![
            server_version,
            task_list,
            task_get,
            task_create,
            task_queue,
            task_start,
            task_pause,
            task_resume,
            task_remove,
            start_event_stream,
            stop_event_stream,
            load_settings,
            save_settings,
            credential_status,
            save_credential,
            clear_credential,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Nexum desktop");
}
