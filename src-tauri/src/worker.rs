//! Background work (spec 5.1 and 5.2): the Downloads watcher and sorting
//! pipeline, the index watchers that keep search current, and the initial
//! scan. Everything here runs on its own threads and reports through events.

use crate::error::Result;
use crate::index;
use crate::ops;
use crate::origin;
use crate::sorter;
use crate::stability::{self, Pending};
use crate::state::Core;
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const CHANGED_EVENT: &str = "desk://changed";
pub const SORTED_EVENT: &str = "desk://sorted";

enum Msg {
    Candidate(PathBuf),
    IndexChange(PathBuf),
}

/// What the renderer hears after a download is filed.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sorted {
    pub name: String,
    pub folder: String,
    pub path: String,
}

/// Starts the watchers and the worker thread. The debouncers move into the
/// thread so they live as long as the app.
pub fn start(core: Arc<Core>, app: AppHandle) -> Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();

    let mut debouncers = Vec::new();
    if let Some(downloads) = core.roots.downloads.clone() {
        let tx_dl = tx.clone();
        let mut debouncer = new_debouncer(Duration::from_secs(2), move |result: DebounceEventResult| match result {
            Ok(events) => {
                for event in events {
                    let _ = tx_dl.send(Msg::Candidate(event.path));
                }
            }
            Err(error) => log::warn!("downloads watcher: {error}"),
        })
        .map_err(|e| crate::error::DeskError::new(crate::error::ErrorCode::Io, e.to_string()))?;
        if let Err(error) = debouncer.watcher().watch(&downloads, RecursiveMode::NonRecursive) {
            log::warn!("cannot watch {}: {error}", downloads.display());
        }
        debouncers.push(debouncer);
    }
    for root in core.roots.all() {
        let tx_idx = tx.clone();
        let mut debouncer = new_debouncer(Duration::from_secs(3), move |result: DebounceEventResult| match result {
            Ok(events) => {
                for event in events {
                    let _ = tx_idx.send(Msg::IndexChange(event.path));
                }
            }
            Err(error) => log::warn!("index watcher: {error}"),
        })
        .map_err(|e| crate::error::DeskError::new(crate::error::ErrorCode::Io, e.to_string()))?;
        if let Err(error) = debouncer.watcher().watch(&root, RecursiveMode::Recursive) {
            log::warn!("cannot watch {}: {error}", root.display());
        }
        debouncers.push(debouncer);
    }

    std::thread::Builder::new()
        .name("desk-worker".into())
        .spawn(move || {
            let _keep_alive = debouncers;
            run(core, app, rx);
        })
        .map_err(crate::error::DeskError::from)?;
    Ok(())
}

fn run(core: Arc<Core>, app: AppHandle, rx: mpsc::Receiver<Msg>) {
    let mut pending: HashMap<PathBuf, Pending> = HashMap::new();
    let mut index_dirty = false;
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Msg::Candidate(path)) => {
                let now = Instant::now();
                match std::fs::metadata(&path) {
                    Ok(meta) if meta.is_file() => {
                        pending.entry(path).and_modify(|p| p.observe(meta.len(), now)).or_insert_with(|| Pending::new(meta.len(), now));
                    }
                    _ => {
                        pending.remove(&path);
                    }
                }
            }
            Ok(Msg::IndexChange(path)) => {
                if let Ok(conn) = core.db() {
                    if let Err(error) = index::refresh_path(&conn, &path) {
                        log::warn!("index refresh {}: {error}", path.display());
                    }
                }
                index_dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        tick(&core, &app, &mut pending);
        if index_dirty {
            index_dirty = false;
            let _ = app.emit(CHANGED_EVENT, ());
        }
    }
}

fn tick(core: &Arc<Core>, app: &AppHandle, pending: &mut HashMap<PathBuf, Pending>) {
    let now = Instant::now();
    let due: Vec<PathBuf> = pending.iter().filter(|(_, p)| now >= p.next_try).map(|(path, _)| path.clone()).collect();
    for path in due {
        let Some(state) = pending.get_mut(&path) else { continue };
        let Ok(meta) = std::fs::metadata(&path) else {
            pending.remove(&path);
            continue;
        };
        state.observe(meta.len(), now);
        if state.expired(now) {
            log::info!("giving up on {}: still busy after a day", path.display());
            if let Ok(conn) = core.db() {
                let _ = ops::note_failure(&conn, &path, "The file stayed open in another program for a day, so it was left in Downloads.");
            }
            pending.remove(&path);
            continue;
        }
        if !(state.is_quiet(now) && stability::can_open_exclusively(&path)) {
            state.defer(now);
            continue;
        }
        pending.remove(&path);
        sort_one(core, app, &path);
    }
}

/// Files one finished download and tells the renderer.
fn sort_one(core: &Arc<Core>, app: &AppHandle, path: &Path) {
    let ext = sorter::extension(path);
    if sorter::stays_in_downloads(&ext) || index::entry_for(path).is_none() {
        return;
    }
    let origin = origin::read_origin(path);
    let kind = sorter::classify(&ext);
    let school = match core.roots.school_files() {
        Ok(s) => s,
        Err(error) => {
            log::error!("no School Files folder: {error}");
            return;
        }
    };
    let dest = sorter::destination(&school, kind, chrono::Local::now());
    let Ok(conn) = core.db() else { return };
    match ops::move_file(&conn, &core.roots, path, &dest, ops::OpKind::AutoSort) {
        Ok(new_path) => {
            if let Err(error) = index::set_origin(&conn, &new_path, &origin, crate::db::now()) {
                log::warn!("origin for {}: {error}", new_path.display());
            }
            let name = new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = format!("School Files > {}", kind.folder_name());
            log::info!("sorted {} into {} (from {})", name, folder, origin.label);
            notify(app, core, &name, &folder);
            let _ = app.emit(SORTED_EVENT, Sorted { name, folder, path: new_path.to_string_lossy().into_owned() });
            let _ = app.emit(CHANGED_EVENT, ());
        }
        Err(error) => {
            log::warn!("could not sort {}: {}", path.display(), error.message);
            let _ = app.emit(CHANGED_EVENT, ());
        }
    }
}

fn notify(app: &AppHandle, core: &Arc<Core>, name: &str, folder: &str) {
    if !crate::commands::settings_of(core).notifications {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    if let Err(error) = app.notification().builder().title("Saved for you").body(format!("{name} is in {folder}.")).show() {
        log::warn!("notification: {error}");
    }
}

/// Walks every root, newest files first, and fills the index in small
/// batches so the database lock is never held long.
pub fn start_scan(core: Arc<Core>, app: AppHandle) {
    let spawned = std::thread::Builder::new().name("desk-scan".into()).spawn(move || {
        core.set_status(|s| {
            s.scanning = true;
            s.scanned = 0;
            s.total = 0;
        });
        let mut entries = Vec::new();
        for root in core.roots.all() {
            index::walk(&root, &mut entries);
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
        let total = entries.len();
        core.set_status(|s| s.total = total);
        let mut scanned = 0;
        for chunk in entries.chunks(25) {
            if let Ok(conn) = core.db() {
                for entry in chunk {
                    if let Err(error) = index::upsert(&conn, entry) {
                        log::warn!("index {}: {error}", entry.path.display());
                    }
                }
            }
            scanned += chunk.len();
            let files = core.db().and_then(|c| index::count(&c)).unwrap_or(0);
            core.set_status(|s| {
                s.scanned = scanned;
                s.files = files;
            });
            if scanned % 200 < 25 {
                let _ = app.emit(CHANGED_EVENT, ());
            }
            // Yield so her laptop stays responsive while the first scan runs.
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Ok(conn) = core.db() {
            for root in core.roots.all() {
                match index::prune_missing(&conn, &root) {
                    Ok(n) if n > 0 => log::info!("pruned {n} missing files under {}", root.display()),
                    Ok(_) => {}
                    Err(error) => log::warn!("prune {}: {error}", root.display()),
                }
            }
        }
        let files = core.db().and_then(|c| index::count(&c)).unwrap_or(0);
        core.set_status(|s| {
            s.scanning = false;
            s.files = files;
        });
        log::info!("initial scan done: {files} files");
        let _ = app.emit(CHANGED_EVENT, ());
    });
    if let Err(error) = spawned {
        log::error!("could not start the scan thread: {error}");
    }
}
