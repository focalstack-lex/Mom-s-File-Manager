//! Desk native core. Every side effect (database, filesystem, shell) lives
//! here behind `#[tauri::command]` functions that validate their input; the
//! renderer only displays what these commands return.

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

use std::sync::Arc;
use tauri::Manager;

/// Builds and runs the application. A startup failure is logged and the
/// process exits with status 1 instead of panicking, because release builds
/// abort on panic with no console.
pub fn run() {
    let log_level = if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    let minimized = std::env::args().any(|a| a == "--minimized");

    let builder = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: Some("desk".into()) })])
                .level(log_level)
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir().map_err(|e| {
                log::error!("no app data folder: {e}");
                e
            })?;
            let opened = match db::open(&data_dir) {
                Ok(opened) => opened,
                Err(error) => {
                    log::error!("Desk could not open its database in {}: {error}", data_dir.display());
                    std::process::exit(1);
                }
            };
            let roots = paths::Roots::detect();
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
            commands::index_status,
        ]);

    if let Err(error) = builder.run(tauri::generate_context!()) {
        log::error!("Desk could not start: {error}");
        std::process::exit(1);
    }
}
