//! Desk native core. Every side effect (database, filesystem, shell) lives
//! here behind `#[tauri::command]` functions that validate their input; the
//! renderer only displays what these commands return.

mod browsers;
mod cleanup;
mod clipboard;
mod commands;
mod db;
mod dupes;
mod error;
mod extract;
mod index;
mod ops;
mod origin;
mod paths;
mod sorter;
mod stability;
mod state;
mod templates;
mod tray;
mod worker;

pub use error::{DeskError, ErrorCode};

// extract.rs relies on catching panics from third-party file readers.
#[cfg(panic = "abort")]
compile_error!("Desk must be built with panic = \"unwind\": one malformed file would otherwise crash the app (see Cargo.toml).");

/// Logs every panic with its location, so a crash in any thread leaves a
/// line in desk.log instead of vanishing.
fn install_panic_logger() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        log::error!("panic in thread {thread}: {info}");
        if cfg!(debug_assertions) {
            default(info);
        }
    }));
}

use std::sync::Arc;
use tauri::Manager;

/// Builds and runs the application. A startup failure is logged and the
/// process exits with status 1 instead of panicking; a panic anywhere is
/// logged (install_panic_logger) and file-reader panics are caught.
pub fn run() {
    // Debug logging in a debug build or during a dry run (DESK_ROOTS), Info otherwise.
    let log_level = if cfg!(debug_assertions) || paths::Roots::test_mode() { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    let minimized = std::env::args().any(|a| a == "--minimized");
    install_panic_logger();

    // A dry run must be able to sit beside the installed app, so it gets its
    // own log file and skips the single-instance rule (which would hand the
    // launch to the running copy and exit).
    let test_mode = paths::Roots::test_mode();
    let log_file = if test_mode { "desk-test" } else { "desk" };

    let mut builder = tauri::Builder::default().plugin(
        tauri_plugin_log::Builder::new()
            .targets([tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: Some(log_file.into()) })])
            .level(log_level)
            .max_file_size(5 * 1024 * 1024)
            .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
            .build(),
    );
    if !test_mode {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show(app)));
    }
    let builder = builder
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // DESK_DATA_DIR keeps a dry run's index away from the real one.
            let data_dir = match std::env::var_os("DESK_DATA_DIR") {
                Some(dir) => std::path::PathBuf::from(dir),
                None => app.path().app_local_data_dir().map_err(|e| {
                    log::error!("no app data folder: {e}");
                    e
                })?,
            };
            let opened = match db::open(&data_dir) {
                Ok(opened) => opened,
                Err(error) => {
                    log::error!("Desk could not open its database in {}: {error}", data_dir.display());
                    std::process::exit(1);
                }
            };
            let roots = paths::Roots::detect(&commands::root_choices(&opened.connection));
            if paths::Roots::test_mode() {
                log::info!("test mode: roots {:?}, data in {}", roots.all(), data_dir.display());
            }
            if let Ok(school) = roots.school_files() {
                if let Err(error) = std::fs::create_dir_all(&school) {
                    log::warn!("cannot create {}: {error}", school.display());
                }
            }
            let core = Arc::new(state::Core {
                db: std::sync::Mutex::new(opened.connection),
                roots,
                status: std::sync::Mutex::new(index::IndexStatus::default()),
                recovered: opened.recovered,
            });
            app.manage(state::AppState(core.clone()));

            // The window is built here rather than in tauri.conf.json so a dry
            // run (DESK_ROOTS plus DESK_DEBUG_PORT) can open the WebView2
            // debugging port for scripts/verify-native.mjs. A normal start never does.
            let window = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Desk")
                .inner_size(1180.0, 760.0)
                .min_inner_size(640.0, 480.0)
                .center()
                .visible(false);
            // WebView2 refuses to share its browser data folder between two
            // processes started with different options, so a dry run beside
            // the installed app keeps its own folder next to its database.
            let window = if test_mode { window.data_directory(data_dir.join("webview")) } else { window };
            #[cfg(windows)]
            let window = match std::env::var("DESK_DEBUG_PORT").ok().filter(|_| test_mode) {
                Some(port) if port.chars().all(|c| c.is_ascii_digit()) => {
                    log::info!("test mode: WebView2 debugging port {port}");
                    window.additional_browser_args(&format!(
                        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
                    ))
                }
                _ => window,
            };
            // A failed window used to end the process with nothing in the log.
            if let Err(error) = window.build() {
                log::error!("the window could not be created: {error}");
                return Err(error.into());
            }

            if let Err(error) = tray::setup(app.handle()) {
                log::warn!("tray icon: {error}");
            }
            let settings = commands::settings_of(&core);
            commands::apply_autostart(app.handle(), settings.start_at_login);
            if let Err(error) = worker::start(core.clone(), app.handle().clone()) {
                log::error!("watchers did not start: {error}");
            }
            worker::start_scan(core, app.handle().clone());
            if !minimized {
                tray::show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides the window; Quit lives in the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::find,
            commands::set_pinned,
            commands::forget_pin,
            commands::open_path,
            commands::reveal_path,
            commands::open_school_files,
            commands::copy_files,
            commands::preview,
            commands::move_files,
            commands::rename_file,
            commands::create_folder,
            commands::recycle_files,
            commands::list_destinations,
            commands::list_activity,
            commands::undo,
            commands::cleanup_report,
            commands::list_templates,
            commands::add_template,
            commands::remove_template,
            commands::use_template,
            commands::get_settings,
            commands::set_settings,
            commands::add_folder,
            commands::remove_folder,
            commands::restart_app,
            commands::browser_check,
            commands::index_status,
        ]);

    if let Err(error) = builder.run(tauri::generate_context!()) {
        log::error!("Desk could not start: {error}");
        std::process::exit(1);
    }
}
