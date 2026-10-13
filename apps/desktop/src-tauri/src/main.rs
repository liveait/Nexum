// Prevents additional console window on Windows in debug mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod download_metadata;
mod download_target;

use desktop_lib::RpcRoute;
use desktop_lib::managed_server::ManagedServer;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ServerMode {
    #[default]
    Managed,
    External,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DesktopSettings {
    #[serde(default)]
    mode: ServerMode,
    server: String,
    refresh_interval_secs: u64,
    #[serde(default)]
    language: DesktopLanguage,
    #[serde(default)]
    download_directory: Option<String>,
    /// Version string from the last time the app was running. Used to detect
    /// upgrades: if current CARGO_PKG_VERSION > last_seen_version, the user
    /// has upgraded and we should re-sync tasks.
    #[serde(default)]
    last_seen_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
enum DesktopLanguage {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            mode: ServerMode::Managed,
            server: "127.0.0.1:39100".to_owned(),
            refresh_interval_secs: 5,
            language: DesktopLanguage::System,
            download_directory: None,
            last_seen_version: None,
        }
    }
}

struct DesktopRuntime {
    settings: Mutex<DesktopSettings>,
    managed: ManagedServer,
    // Serializes mode changes with task target allocation and creation.
    creation: Mutex<()>,
}

impl DesktopRuntime {
    fn settings(&self) -> DesktopSettings {
        self.settings
            .lock()
            .expect("settings mutex poisoned")
            .clone()
    }

    fn route(&self) -> Result<RpcRoute, String> {
        let settings = self.settings();
        match settings.mode {
            ServerMode::Managed => self.managed.route(),
            ServerMode::External => Ok(RpcRoute::external(settings.server)),
        }
    }

    fn external_server(&self) -> Result<String, String> {
        let settings = self.settings();
        if settings.mode != ServerMode::External {
            return Err("credentials are configured only for an external Server".to_owned());
        }
        Ok(settings.server)
    }
}

#[derive(Serialize)]
struct ConnectionStatus {
    mode: ServerMode,
    state: String,
    generation: u64,
    error: Option<String>,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("could not resolve application settings directory: {error}"))
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve application data directory: {error}"))
}

/// Checks whether this is a fresh install: no settings file and no server
/// data directory. A fresh install has never been run before.
fn is_fresh_install(app: &AppHandle) -> Result<bool, String> {
    let settings = settings_path(app)?;
    if settings.is_file() {
        return Ok(false);
    }
    let server_dir = data_dir(app)?.join("server");
    if server_dir.is_dir() {
        return Ok(false);
    }
    Ok(true)
}

fn read_settings(app: &AppHandle) -> Result<DesktopSettings, String> {
    let path = settings_path(app)?;
    if !path.is_file() {
        return Ok(DesktopSettings::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read application settings: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("could not parse application settings: {error}"))
}

fn write_settings(app: &AppHandle, settings: &DesktopSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let directory = path
        .parent()
        .ok_or_else(|| "application settings path has no parent".to_owned())?;
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create application settings directory: {error}"))?;
    let content = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("could not serialize settings: {error}"))?;
    let temporary = directory.join(format!(".settings-{}.tmp", std::process::id()));
    std::fs::write(&temporary, content)
        .map_err(|error| format!("could not write settings: {error}"))?;
    std::fs::rename(&temporary, &path).map_err(|error| format!("could not save settings: {error}"))
}

#[tauri::command]
fn load_settings(state: tauri::State<'_, Arc<DesktopRuntime>>) -> DesktopSettings {
    state.settings()
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    mut settings: DesktopSettings,
) -> Result<(), String> {
    settings.server = settings.server.trim().to_owned();
    if settings.server.is_empty() || settings.refresh_interval_secs > 60 {
        return Err("server address and refresh interval must be valid".to_owned());
    }
    if let Some(directory) = settings.download_directory.as_ref() {
        let directory = download_target::local_directory(Path::new(directory))?;
        settings.download_directory = Some(directory.to_string_lossy().into_owned());
    }
    let _creation = state
        .creation
        .lock()
        .expect("download creation mutex poisoned");
    let previous_mode = state.settings().mode;
    write_settings(&app, &settings)?;
    // On first save, record the current version for upgrade detection.
    if settings.last_seen_version.is_none() {
        settings.last_seen_version = Some(env!("CARGO_PKG_VERSION").to_owned());
    }
    let next_mode = settings.mode;
    *state.settings.lock().expect("settings mutex poisoned") = settings;
    if previous_mode != next_mode {
        match next_mode {
            ServerMode::Managed => state.managed.enable()?,
            ServerMode::External => state.managed.disable()?,
        }
    }
    Ok(())
}

#[tauri::command]
fn check_fresh_install(app: AppHandle) -> Result<bool, String> {
    is_fresh_install(&app)
}

#[tauri::command]
fn get_upgrade_info(
    _app: AppHandle,
    state: tauri::State<'_, Arc<DesktopRuntime>>,
) -> Result<Option<Value>, String> {
    let settings = state.settings();
    let current_version = env!("CARGO_PKG_VERSION");
    match settings.last_seen_version {
        Some(last) if last != current_version => {
            // User upgraded — return upgrade info for the frontend to display.
            Ok(Some(serde_json::json!({
                "upgraded": true,
                "old_version": last,
                "new_version": current_version,
            })))
        }
        _ => Ok(None),
    }
}

#[tauri::command]
fn connection_status(state: tauri::State<'_, Arc<DesktopRuntime>>) -> ConnectionStatus {
    let mode = state.settings().mode;
    let status = state.managed.status();
    ConnectionStatus {
        mode,
        state: if mode == ServerMode::External {
            "external".to_owned()
        } else {
            status.state
        },
        generation: status.generation,
        error: if mode == ServerMode::External {
            None
        } else {
            status.error
        },
    }
}

#[tauri::command]
fn retry_managed_server(state: tauri::State<'_, Arc<DesktopRuntime>>) -> Result<(), String> {
    let _transition = state
        .creation
        .lock()
        .expect("download creation mutex poisoned");
    if state.settings().mode != ServerMode::Managed {
        return Err("local Server retry is only available in managed mode".to_owned());
    }
    state.managed.retry()
}

#[tauri::command]
fn credential_status(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
) -> Result<Option<String>, String> {
    desktop_lib::credential_status(&state.external_server()?)
}

#[tauri::command]
fn save_credential(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    scheme: String,
    secret: String,
) -> Result<(), String> {
    desktop_lib::save_credential(&state.external_server()?, &scheme, secret)
}

#[tauri::command]
fn clear_credential(state: tauri::State<'_, Arc<DesktopRuntime>>) -> Result<(), String> {
    desktop_lib::clear_credential(&state.external_server()?)
}

#[tauri::command]
fn server_version(state: tauri::State<'_, Arc<DesktopRuntime>>) -> Result<String, String> {
    state
        .route()?
        .call("server.version", None, 5000)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "server returned an invalid protocol version".to_owned())
}

#[tauri::command]
fn task_list(state: tauri::State<'_, Arc<DesktopRuntime>>) -> Result<Value, String> {
    state.route()?.call("task.list", None, 5000)
}

#[tauri::command]
fn task_get(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    task_id: String,
) -> Result<Value, String> {
    state
        .route()?
        .call("task.get", Some(json!({"id":task_id})), 5000)
}

fn is_local_server(server: &str) -> bool {
    let server = server.strip_prefix("tls://").unwrap_or(server);
    server
        .parse::<std::net::SocketAddr>()
        .is_ok_and(|address| address.ip().is_loopback() && address.port() != 0)
        || server.rsplit_once(':').is_some_and(|(host, port)| {
            host.eq_ignore_ascii_case("localhost")
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        })
}

#[tauri::command]
fn task_create(
    app: AppHandle,
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    source: String,
    filename: String,
    directory: String,
) -> Result<Value, String> {
    let source = download_target::validate_source(&source)?;
    download_target::validate_filename(&filename)?;
    let _creation = state
        .creation
        .lock()
        .expect("download creation mutex poisoned");
    let settings = state.settings();
    let local = settings.mode == ServerMode::Managed || is_local_server(&settings.server);
    let directory = if directory.trim().is_empty() && local {
        settings
            .download_directory
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(|| {
                app.path().download_dir().map_err(|error| {
                    format!("could not resolve Downloads; choose a folder: {error}")
                })
            })?
    } else {
        PathBuf::from(directory)
    };
    let directory = if local {
        download_target::local_directory(&directory)?
    } else {
        directory
    };
    if settings.mode == ServerMode::Managed {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("server");
        if data_dir
            .canonicalize()
            .is_ok_and(|data_dir| directory.starts_with(data_dir))
        {
            return Err(
                "choose a download folder outside the local service data directory".to_owned(),
            );
        }
    }
    let route = state.route()?;
    let tasks = route.call("task.list", None, 5000)?;
    let tasks = tasks
        .as_array()
        .ok_or_else(|| "server returned an invalid task list".to_owned())?;
    let destination = if local {
        download_target::allocate(&directory, &filename, tasks, true)?
            .into_os_string()
            .into_string()
            .map_err(|_| "download destination must be valid UTF-8".to_owned())?
    } else {
        download_target::allocate_remote(
            directory
                .to_str()
                .ok_or_else(|| "download destination must be valid UTF-8".to_owned())?,
            &filename,
            tasks,
        )?
    };
    route.call("task.create", Some(json!({"id":download_target::random_task_id()?,"source":source,"destination":destination})), 30000)
}

fn task_action(state: &DesktopRuntime, method: &str, task_id: String) -> Result<bool, String> {
    state
        .route()?
        .call(method, Some(json!({"id":task_id})), 5000)?
        .as_bool()
        .ok_or_else(|| "server returned an invalid boolean result".to_owned())
}

#[tauri::command]
fn task_queue(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    task_id: String,
) -> Result<bool, String> {
    task_action(&state, "task.queue", task_id)
}
#[tauri::command]
fn task_pause(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    task_id: String,
) -> Result<bool, String> {
    task_action(&state, "task.pause", task_id)
}
#[tauri::command]
fn task_resume(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    task_id: String,
) -> Result<bool, String> {
    task_action(&state, "task.resume", task_id)
}
#[tauri::command]
fn task_remove(
    state: tauri::State<'_, Arc<DesktopRuntime>>,
    task_id: String,
) -> Result<bool, String> {
    task_action(&state, "task.remove", task_id)
}
#[tauri::command]
fn task_start(state: tauri::State<'_, Arc<DesktopRuntime>>) -> Result<String, String> {
    state
        .route()?
        .call("task.start", None, 5000)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "server returned an invalid task ID".to_owned())
}

#[tauri::command]
fn start_event_stream(
    app: AppHandle,
    events: tauri::State<'_, desktop_lib::EventSubscriptionManager>,
    state: tauri::State<'_, Arc<DesktopRuntime>>,
) -> Result<u64, String> {
    let runtime = Arc::clone(&state);
    let context = runtime.settings();
    let label = if context.mode == ServerMode::Managed {
        "managed".to_owned()
    } else {
        context.server.clone()
    };
    // A stale subscription never follows a settings switch to a different context.
    Ok(events.start_resolved(
        app,
        label,
        Arc::new(move || {
            let _transition = runtime
                .creation
                .lock()
                .expect("download creation mutex poisoned");
            let current = runtime.settings();
            if current.mode != context.mode
                || current.mode == ServerMode::External && current.server != context.server
            {
                return Err("connection settings changed".to_owned());
            }
            runtime.route()
        }),
    ))
}

#[tauri::command]
fn stop_event_stream(
    events: tauri::State<'_, desktop_lib::EventSubscriptionManager>,
    generation: u64,
) {
    events.stop(generation);
}

#[tauri::command]
async fn suggest_download_filename(source: String) -> Option<String> {
    download_metadata::suggest_download_filename(source).await
}

/// Saves partial download metadata so an interrupted task can be resumed.
#[tauri::command]
fn save_partial_metadata(
    app: tauri::AppHandle,
    task_id: String,
    source: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> Result<String, String> {
    let server_data_dir = data_dir(&app)?.join("server");
    if !server_data_dir.is_dir() {
        return Err("server data directory not found; start a server first".to_owned());
    }
    download_metadata::save_partial_metadata(
        &server_data_dir,
        &task_id,
        &source,
        downloaded_bytes,
        total_bytes,
    )
    .map(|p| p.to_string_lossy().to_string())
}

/// Retrieves partial metadata for a task to reconstruct an interrupted download.
#[tauri::command]
fn read_partial_metadata(
    app: tauri::AppHandle,
    task_id: String,
) -> Result<Option<download_metadata::PartialMetadata>, String> {
    let server_data_dir = data_dir(&app)?.join("server");
    if !server_data_dir.is_dir() {
        return Ok(None);
    }
    download_metadata::read_partial_metadata(&server_data_dir, &task_id)
}

/// Removes partial metadata for a completed task.
#[tauri::command]
fn clear_partial_metadata(app: tauri::AppHandle, task_id: String) -> Result<(), String> {
    let server_data_dir = data_dir(&app)?.join("server");
    if !server_data_dir.is_dir() {
        return Ok(());
    }
    download_metadata::clear_partial_metadata(&server_data_dir, &task_id)
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(desktop_lib::EventSubscriptionManager::default())
        .setup(|app| {
            let settings = read_settings(app.handle())?;
            let binary = if cfg!(debug_assertions) && !env!("NEXUM_SERVER_DEV_PATH").is_empty() {
                PathBuf::from(env!("NEXUM_SERVER_DEV_PATH"))
            } else {
                app.path().resource_dir()?.join("nexum-server")
            };
            let managed = ManagedServer::new(
                binary,
                env!("NEXUM_SERVER_SHA256").to_owned(),
                app.path().app_data_dir()?.join("server"),
            );
            if settings.mode == ServerMode::Managed {
                // Supervisor failures remain visible in Settings rather than
                // preventing the application window from opening.
                let _ = managed.enable();
            }
            app.manage(Arc::new(DesktopRuntime {
                settings: Mutex::new(settings),
                managed,
                creation: Mutex::new(()),
            }));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
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
            connection_status,
            retry_managed_server,
            suggest_download_filename,
            credential_status,
            save_credential,
            clear_credential,
            save_partial_metadata,
            read_partial_metadata,
            clear_partial_metadata,
            check_fresh_install,
            get_upgrade_info,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Nexum desktop");
    app.run(|app, event| match event {
        tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
            if let Some(state) = app.try_state::<Arc<DesktopRuntime>>() {
                let _ = state.managed.shutdown();
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_managed_external_and_language() {
        let mut settings = DesktopSettings::default();
        assert_eq!(settings.mode, ServerMode::Managed);
        settings.mode = ServerMode::External;
        settings.language = DesktopLanguage::SimplifiedChinese;
        let restored: DesktopSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.mode, ServerMode::External);
        assert_eq!(restored.language, DesktopLanguage::SimplifiedChinese);
        assert!(restored.last_seen_version.is_none());
    }
    #[test]
    fn local_filesystem_checks_are_only_for_explicit_loopback_addresses() {
        for server in [
            "127.0.0.1:39100",
            "[::1]:39100",
            "localhost:39100",
            "tls://localhost:39100",
        ] {
            assert!(is_local_server(server));
        }
        for server in [
            "example.com:39100",
            "tls://example.com:443",
            "localhost.evil:39100",
            "localhost:0",
        ] {
            assert!(!is_local_server(server));
        }
    }
}
