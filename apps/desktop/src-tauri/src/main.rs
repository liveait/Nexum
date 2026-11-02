// Prevents additional console window on Windows in debug mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about_links;
mod download_metadata;
mod download_target;

use desktop_lib::RpcRoute;
use desktop_lib::managed_server::ManagedServer;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

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
    /// Backend-owned version recorded on first save or explicit upgrade acknowledgement.
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
    first_run: FirstRunState,
    managed: ManagedServer,
    // Serializes settings commits/acknowledgement with mode changes and task creation.
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

struct FirstRunState {
    fresh: bool,
}

impl FirstRunState {
    fn capture(settings: &Path, server_dir: &Path) -> Self {
        Self {
            fresh: !settings.is_file() && !server_dir.is_dir(),
        }
    }

    fn is_fresh(&self) -> bool {
        self.fresh
    }
}

fn read_settings(app: &AppHandle) -> Result<DesktopSettings, String> {
    read_settings_at(&settings_path(app)?)
}

fn read_settings_at(path: &Path) -> Result<DesktopSettings, String> {
    if !path.is_file() {
        return Ok(DesktopSettings::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read application settings: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("could not parse application settings: {error}"))
}

fn write_settings(app: &AppHandle, settings: &DesktopSettings) -> Result<(), String> {
    write_settings_at(&settings_path(app)?, settings)
}

fn write_settings_at(path: &Path, settings: &DesktopSettings) -> Result<(), String> {
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
    std::fs::rename(&temporary, path).map_err(|error| format!("could not save settings: {error}"))
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
    let next_mode = settings.mode;
    persist_user_settings(
        &state.settings,
        settings,
        env!("CARGO_PKG_VERSION"),
        |next| write_settings(&app, next),
    )?;
    if previous_mode != next_mode {
        match next_mode {
            ServerMode::Managed => state.managed.enable()?,
            ServerMode::External => state.managed.disable()?,
        }
    }
    Ok(())
}

fn persist_user_settings(
    current: &Mutex<DesktopSettings>,
    mut settings: DesktopSettings,
    current_version: &str,
    write: impl FnOnce(&DesktopSettings) -> Result<(), String>,
) -> Result<(), String> {
    let mut stored = current.lock().expect("settings mutex poisoned");
    // Frontend preferences never own or acknowledge this internal metadata.
    settings.last_seen_version = stored
        .last_seen_version
        .clone()
        .or_else(|| Some(current_version.to_owned()));
    write(&settings)?;
    *stored = settings;
    Ok(())
}

fn persist_upgrade_acknowledgement(
    current: &Mutex<DesktopSettings>,
    current_version: &str,
    write: impl FnOnce(&DesktopSettings) -> Result<(), String>,
) -> Result<(), String> {
    let mut stored = current.lock().expect("settings mutex poisoned");
    if stored.last_seen_version.as_deref() == Some(current_version) {
        return Ok(());
    }
    let mut next = stored.clone();
    next.last_seen_version = Some(current_version.to_owned());
    write(&next)?;
    *stored = next;
    Ok(())
}

#[tauri::command]
fn acknowledge_upgrade(
    app: AppHandle,
    state: tauri::State<'_, Arc<DesktopRuntime>>,
) -> Result<(), String> {
    let _creation = state
        .creation
        .lock()
        .expect("download creation mutex poisoned");
    persist_upgrade_acknowledgement(&state.settings, env!("CARGO_PKG_VERSION"), |next| {
        write_settings(&app, next)
    })
}

#[tauri::command]
fn check_fresh_install(state: tauri::State<'_, Arc<DesktopRuntime>>) -> bool {
    state.first_run.is_fresh()
}

#[tauri::command]
fn get_upgrade_info(
    _app: AppHandle,
    state: tauri::State<'_, Arc<DesktopRuntime>>,
) -> Result<Option<Value>, String> {
    Ok(upgrade_info(state.settings(), env!("CARGO_PKG_VERSION")))
}

fn upgrade_info(settings: DesktopSettings, current_version: &str) -> Option<Value> {
    let last = settings.last_seen_version?;
    let previous = semver::Version::parse(&last).ok()?;
    let current = semver::Version::parse(current_version).ok()?;
    if current.cmp_precedence(&previous).is_gt() {
        Some(serde_json::json!({
                "upgraded": true,
                "old_version": last,
                "new_version": current_version,
        }))
    } else {
        None
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

#[tauri::command]
fn open_project_link(app: AppHandle, link: about_links::ProjectLink) -> Result<(), String> {
    about_links::open_project_link(link, |url| app.opener().open_url(url, None::<&str>))
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(desktop_lib::EventSubscriptionManager::default())
        .setup(|app| {
            let settings = read_settings(app.handle())?;
            let server_data_dir = data_dir(app.handle())?.join("server");
            // Capture before the asynchronous supervisor can create Server data.
            let first_run = FirstRunState::capture(&settings_path(app.handle())?, &server_data_dir);
            let binary = if cfg!(debug_assertions) && !env!("NEXUM_SERVER_DEV_PATH").is_empty() {
                PathBuf::from(env!("NEXUM_SERVER_DEV_PATH"))
            } else {
                app.path().resource_dir()?.join("nexum-server")
            };
            let managed = ManagedServer::new(
                binary,
                env!("NEXUM_SERVER_SHA256").to_owned(),
                server_data_dir,
            );
            if settings.mode == ServerMode::Managed {
                // Supervisor failures remain visible in Settings rather than
                // preventing the application window from opening.
                let _ = managed.enable();
            }
            app.manage(Arc::new(DesktopRuntime {
                settings: Mutex::new(settings),
                first_run,
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
            acknowledge_upgrade,
            open_project_link,
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

    struct SettingsDirectory(PathBuf);

    impl SettingsDirectory {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(download_target::random_task_id().unwrap());
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn path(&self) -> PathBuf {
            self.0.join(SETTINGS_FILE)
        }
    }

    impl Drop for SettingsDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_run_capture_is_read_only() {
        let directory = SettingsDirectory::new();
        let settings = directory.path();
        let server = directory.0.join("server");
        let startup = FirstRunState::capture(&settings, &server);
        assert!(startup.is_fresh());
        assert!(!settings.exists());
        assert!(!server.exists());
    }

    #[test]
    fn first_run_server_startup_does_not_erase_initial_eligibility() {
        let directory = SettingsDirectory::new();
        let server = directory.0.join("server");
        let startup = FirstRunState::capture(&directory.path(), &server);
        // Managed startup can create its data directory before the first RPC.
        std::fs::create_dir(&server).unwrap();
        std::fs::write(server.join("data.lock"), b"owned fixture").unwrap();
        assert!(startup.is_fresh());
    }

    #[test]
    fn first_run_preference_save_does_not_reclassify_current_startup() {
        let directory = SettingsDirectory::new();
        let startup = FirstRunState::capture(&directory.path(), &directory.0.join("server"));
        let settings = Mutex::new(DesktopSettings::default());
        persist_user_settings(&settings, DesktopSettings::default(), "0.1.0", |next| {
            write_settings_at(&directory.path(), next)
        })
        .unwrap();
        assert!(startup.is_fresh());
        assert_eq!(
            read_settings_at(&directory.path())
                .unwrap()
                .last_seen_version
                .as_deref(),
            Some("0.1.0")
        );
    }

    #[test]
    fn first_run_existing_settings_in_both_modes_are_not_fresh() {
        for mode in [ServerMode::Managed, ServerMode::External] {
            let directory = SettingsDirectory::new();
            let settings = DesktopSettings {
                mode,
                ..DesktopSettings::default()
            };
            write_settings_at(&directory.path(), &settings).unwrap();
            let before = std::fs::read(directory.path()).unwrap();
            let server = directory.0.join("server");
            let startup = FirstRunState::capture(&directory.path(), &server);
            assert!(!startup.is_fresh());
            assert!(!server.exists());
            assert_eq!(std::fs::read(directory.path()).unwrap(), before);
        }
    }

    #[test]
    fn first_run_existing_server_without_settings_is_not_fresh() {
        let directory = SettingsDirectory::new();
        let server = directory.0.join("server");
        std::fs::create_dir(&server).unwrap();
        let startup = FirstRunState::capture(&directory.path(), &server);
        assert!(!startup.is_fresh());
        assert!(!directory.path().exists());
    }

    #[test]
    fn first_run_existing_profile_cannot_become_fresh_in_the_same_runtime() {
        let directory = SettingsDirectory::new();
        write_settings_at(&directory.path(), &DesktopSettings::default()).unwrap();
        let startup = FirstRunState::capture(&directory.path(), &directory.0.join("server"));
        std::fs::remove_file(directory.path()).unwrap();
        assert!(!startup.is_fresh());
    }

    #[test]
    fn upgrade_record_first_save_persists_the_actual_version() {
        let directory = SettingsDirectory::new();
        let state = Mutex::new(DesktopSettings::default());
        persist_user_settings(&state, DesktopSettings::default(), "0.1.0", |next| {
            write_settings_at(&directory.path(), next)
        })
        .unwrap();
        let restarted = read_settings_at(&directory.path()).unwrap();
        assert_eq!(restarted.last_seen_version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn upgrade_record_ordinary_save_preserves_unacknowledged_backend_version() {
        let directory = SettingsDirectory::new();
        let state = Mutex::new(DesktopSettings {
            last_seen_version: Some("0.0.9".to_owned()),
            ..DesktopSettings::default()
        });
        persist_user_settings(&state, DesktopSettings::default(), "0.1.0", |next| {
            write_settings_at(&directory.path(), next)
        })
        .unwrap();
        let restarted = read_settings_at(&directory.path()).unwrap();
        assert_eq!(restarted.last_seen_version.as_deref(), Some("0.0.9"));
        assert!(upgrade_info(restarted, "0.1.0").is_some());
        assert_eq!(
            state.lock().unwrap().last_seen_version.as_deref(),
            Some("0.0.9")
        );
    }

    #[test]
    fn upgrade_record_frontend_cannot_forge_internal_version() {
        let directory = SettingsDirectory::new();
        let state = Mutex::new(DesktopSettings::default());
        let submitted = DesktopSettings {
            last_seen_version: Some("99.0.0".to_owned()),
            ..DesktopSettings::default()
        };
        persist_user_settings(&state, submitted, "0.1.0", |next| {
            write_settings_at(&directory.path(), next)
        })
        .unwrap();
        assert_eq!(
            read_settings_at(&directory.path())
                .unwrap()
                .last_seen_version
                .as_deref(),
            Some("0.1.0")
        );
    }

    #[test]
    fn upgrade_record_downgrade_is_not_an_upgrade() {
        assert!(
            upgrade_info(
                DesktopSettings {
                    last_seen_version: Some("0.2.0".to_owned()),
                    ..DesktopSettings::default()
                },
                "0.1.0"
            )
            .is_none()
        );
    }

    #[test]
    fn upgrade_record_unknown_version_is_not_an_upgrade() {
        assert!(
            upgrade_info(
                DesktopSettings {
                    last_seen_version: Some("unknown".to_owned()),
                    ..DesktopSettings::default()
                },
                "0.1.0"
            )
            .is_none()
        );
    }

    #[test]
    fn upgrade_record_semver_precedence_ignores_build_metadata_and_unknown_versions() {
        for (previous, current, upgraded) in [
            (Some("0.0.9"), "0.1.0", true),
            (Some("0.1.0"), "0.1.0", false),
            (Some("0.2.0"), "0.1.0", false),
            (Some("0.1.0+one"), "0.1.0+two", false),
            (Some("0.1.0-rc.2"), "0.1.0", true),
            (Some("0.1.0-rc.2"), "0.1.0-rc.10", true),
            (Some("0.1.0"), "0.1.0-rc.1", false),
            (Some("0.1"), "0.1.0", false),
            (Some("garbage"), "0.1.0", false),
            (Some("0.1.0"), "unknown", false),
            (None, "0.1.0", false),
        ] {
            let settings = DesktopSettings {
                last_seen_version: previous.map(str::to_owned),
                ..DesktopSettings::default()
            };
            assert_eq!(
                upgrade_info(settings, current).is_some(),
                upgraded,
                "{previous:?} -> {current}"
            );
        }
    }

    #[test]
    fn upgrade_record_submitted_metadata_never_clears_or_acknowledges_pending_upgrade() {
        let directory = SettingsDirectory::new();
        let state = Mutex::new(DesktopSettings {
            last_seen_version: Some("0.0.9".to_owned()),
            ..DesktopSettings::default()
        });
        for submitted in [None, Some("0.1.0"), Some("99.0.0")] {
            let mut value =
                json!({"server":"localhost:39101", "refresh_interval_secs":9, "language":"en"});
            if let Some(version) = submitted {
                value["last_seen_version"] = json!(version);
            }
            let settings = serde_json::from_value(value).unwrap();
            persist_user_settings(&state, settings, "0.1.0", |next| {
                write_settings_at(&directory.path(), next)
            })
            .unwrap();
            let restored = read_settings_at(&directory.path()).unwrap();
            assert_eq!(restored.last_seen_version.as_deref(), Some("0.0.9"));
            assert_eq!(restored.language, DesktopLanguage::English);
            assert_eq!(restored.server, "localhost:39101");
            assert_eq!(
                upgrade_info(restored, "0.1.0").unwrap()["old_version"],
                "0.0.9"
            );
        }
    }

    #[test]
    fn upgrade_record_acknowledgement_persists_survives_restart_and_is_idempotent() {
        let directory = SettingsDirectory::new();
        let initial = DesktopSettings {
            last_seen_version: Some("0.0.9".to_owned()),
            language: DesktopLanguage::English,
            server: "localhost:39102".to_owned(),
            download_directory: Some("/private/example".to_owned()),
            ..DesktopSettings::default()
        };
        write_settings_at(&directory.path(), &initial).unwrap();
        let state = Mutex::new(initial);
        persist_upgrade_acknowledgement(&state, "0.1.0", |next| {
            write_settings_at(&directory.path(), next)
        })
        .unwrap();
        let restarted = read_settings_at(&directory.path()).unwrap();
        assert!(upgrade_info(restarted.clone(), "0.1.0").is_none());
        assert_eq!(restarted.last_seen_version.as_deref(), Some("0.1.0"));
        assert_eq!(restarted.language, DesktopLanguage::English);
        assert_eq!(restarted.server, "localhost:39102");
        assert_eq!(
            restarted.download_directory.as_deref(),
            Some("/private/example")
        );
        let state = Mutex::new(restarted);
        persist_upgrade_acknowledgement(&state, "0.1.0", |_| {
            panic!("idempotent acknowledgement must not write")
        })
        .unwrap();
    }

    #[test]
    fn upgrade_record_write_failures_keep_memory_disk_and_pending_notice_unchanged() {
        let directory = SettingsDirectory::new();
        let original = DesktopSettings {
            last_seen_version: Some("0.0.9".to_owned()),
            language: DesktopLanguage::English,
            ..DesktopSettings::default()
        };
        write_settings_at(&directory.path(), &original).unwrap();
        let before = std::fs::read(directory.path()).unwrap();
        let state = Mutex::new(original.clone());
        let blocked = directory
            .0
            .join(format!(".settings-{}.tmp", std::process::id()));
        std::fs::create_dir(&blocked).unwrap();
        assert!(
            persist_upgrade_acknowledgement(&state, "0.1.0", |next| write_settings_at(
                &directory.path(),
                next
            ))
            .is_err()
        );
        assert!(
            persist_user_settings(&state, DesktopSettings::default(), "0.1.0", |next| {
                write_settings_at(&directory.path(), next)
            })
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(state.lock().unwrap().clone()).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert_eq!(std::fs::read(directory.path()).unwrap(), before);
        assert!(upgrade_info(state.lock().unwrap().clone(), "0.1.0").is_some());
        std::fs::remove_dir(blocked).unwrap();
        persist_upgrade_acknowledgement(&state, "0.1.0", |next| {
            write_settings_at(&directory.path(), next)
        })
        .unwrap();
        assert_eq!(
            read_settings_at(&directory.path()).unwrap().language,
            DesktopLanguage::English
        );
    }

    #[test]
    fn upgrade_record_failed_first_save_leaves_version_unrecorded() {
        let state = Mutex::new(DesktopSettings::default());
        assert!(
            persist_user_settings(&state, DesktopSettings::default(), "0.1.0", |_| Err(
                "storage failed".to_owned()
            ))
            .is_err()
        );
        assert!(state.lock().unwrap().last_seen_version.is_none());
    }

    #[test]
    fn upgrade_record_concurrent_save_and_acknowledgement_preserve_latest_preferences() {
        let directory = SettingsDirectory::new();
        let state = Arc::new(Mutex::new(DesktopSettings {
            last_seen_version: Some("0.0.9".to_owned()),
            ..DesktopSettings::default()
        }));
        let creation = Arc::new(Mutex::new(()));
        let start = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for acknowledge in [true, false] {
            let state = state.clone();
            let creation = creation.clone();
            let start = start.clone();
            let path = directory.path();
            workers.push(std::thread::spawn(move || {
                start.wait();
                let _creation = creation.lock().unwrap();
                if acknowledge {
                    persist_upgrade_acknowledgement(&state, "0.1.0", |next| {
                        write_settings_at(&path, next)
                    })
                    .unwrap();
                } else {
                    let submitted = DesktopSettings {
                        language: DesktopLanguage::English,
                        server: "localhost:39103".to_owned(),
                        ..DesktopSettings::default()
                    };
                    persist_user_settings(&state, submitted, "0.1.0", |next| {
                        write_settings_at(&path, next)
                    })
                    .unwrap();
                }
            }));
        }
        start.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let restored = read_settings_at(&directory.path()).unwrap();
        assert_eq!(restored.last_seen_version.as_deref(), Some("0.1.0"));
        assert_eq!(restored.language, DesktopLanguage::English);
        assert_eq!(restored.server, "localhost:39103");
        assert_eq!(
            serde_json::to_value(state.lock().unwrap().clone()).unwrap(),
            serde_json::to_value(restored).unwrap()
        );
    }

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
