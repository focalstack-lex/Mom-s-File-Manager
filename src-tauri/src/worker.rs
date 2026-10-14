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

pub const CHANGED_EVENT: &str = "tote://changed";
pub const SORTED_EVENT: &str = "tote://sorted";

/// A stamped file created this recently, anywhere Tote watches, counts as a download.
const LIVE_DOWNLOAD_WINDOW_SECS: i64 = 10 * 60;
/// On the first scan, stamped files from the last month fill Recent downloads.
const SCAN_DOWNLOAD_WINDOW_SECS: i64 = 30 * 86_400;

/// Windows reports a write to `file.mp3:Zone.Identifier` with the stream
/// name attached; the file itself is what matters.
fn without_stream(path: PathBuf) -> PathBuf {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return path };
    match name.split_once(':') {
        Some((file, _stream)) if !file.is_empty() => path.with_file_name(file),
        _ => path,
    }
}

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
        .map_err(|e| crate::error::ToteError::new(crate::error::ErrorCode::Io, e.to_string()))?;
        match debouncer.watcher().watch(&downloads, RecursiveMode::NonRecursive) {
            Ok(()) => log::info!("watching downloads at {}", downloads.display()),
            Err(error) => log::warn!("cannot watch {}: {error}", downloads.display()),
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
        .map_err(|e| crate::error::ToteError::new(crate::error::ErrorCode::Io, e.to_string()))?;
        match debouncer.watcher().watch(&root, RecursiveMode::Recursive) {
            Ok(()) => log::info!("watching {}", root.display()),
            Err(error) => log::warn!("cannot watch {}: {error}", root.display()),
        }
        debouncers.push(debouncer);
    }

    std::thread::Builder::new()
        .name("tote-worker".into())
        .spawn(move || {
            let _keep_alive = debouncers;
            run(core, app, rx);
        })
        .map_err(crate::error::ToteError::from)?;
    Ok(())
}

fn run(core: Arc<Core>, app: AppHandle, rx: mpsc::Receiver<Msg>) {
    let mut pending: HashMap<PathBuf, Pending> = HashMap::new();
    let mut index_dirty = false;
    let roots = core.roots.all();
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Msg::Candidate(path)) => {
                let path = without_stream(path);
                log::debug!("download candidate {}", path.display());
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
                let path = without_stream(path);
                log::debug!("index change {}", path.display());
                if index::under_skipped_dir(&path, &roots) {
                    continue;
                }
                if let Err(error) = refresh_unlocked(&core, &path) {
                    log::warn!("index refresh {}: {error}", path.display());
                }
                if let Ok(conn) = core.db() {
                    // A browser that saved outside Downloads still stamps the
                    // file; it joins Recent downloads where it is (spec 5.1.3).
                    match index::note_if_downloaded(&conn, &path, LIVE_DOWNLOAD_WINDOW_SECS) {
                        Ok(true) => log::info!("noticed a download saved outside Downloads: {}", path.display()),
                        Ok(false) => {}
                        Err(error) => log::warn!("download check {}: {error}", path.display()),
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
        let quiet = state.is_quiet(now);
        let free = quiet && stability::can_open_exclusively(&path);
        if !free {
            state.defer(now);
            log::debug!(
                "waiting on {} ({}), attempt {}, next look in {:?}",
                path.display(),
                if quiet { "another program has it open" } else { "still growing" },
                state.attempts,
                state.next_try.saturating_duration_since(now)
            );
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
    // Settings are read before the database lock is taken: the mutex is not
    // reentrant, and reading them inside the locked block deadlocked the worker.
    let notifications = crate::commands::settings_of(core).notifications;
    let moved = {
        let Ok(conn) = core.db() else { return };
        ops::move_file(&conn, &core.roots, path, &dest, ops::OpKind::AutoSort).inspect(|new_path| {
            if let Err(error) = index::set_origin(&conn, new_path, &origin, crate::db::now()) {
                log::warn!("origin for {}: {error}", new_path.display());
            }
        })
    };
    match moved {
        Ok(new_path) => {
            let name = new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = format!("School Files > {}", kind.folder_name());
            log::info!("sorted {} into {} (from {})", name, folder, origin.label);
            if notifications {
                notify(app, &name, &folder);
            }
            let _ = app.emit(SORTED_EVENT, Sorted { name, folder, path: new_path.to_string_lossy().into_owned() });
            let _ = app.emit(CHANGED_EVENT, ());
        }
        Err(error) => {
            log::warn!("could not sort {}: {}", path.display(), error.message);
            let _ = app.emit(CHANGED_EVENT, ());
        }
    }
}

fn notify(app: &AppHandle, name: &str, folder: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(error) = app.notification().builder().title("Saved for you").body(format!("{name} is in {folder}.")).show() {
        log::warn!("notification: {error}");
    }
}

/// Indexes one file while holding the database lock only for two short
/// lookups: the slow part (reading the words inside the file) runs with no
/// lock, so a search never waits on a PDF. The 18,000-file scan on Lex's
/// machine kept the Find screen on "Searching" because the old upsert held
/// the lock across extraction (2026-10-12).
fn index_entry(core: &Arc<Core>, entry: &index::Entry) -> crate::error::Result<i64> {
    let change = {
        let conn = core.db()?;
        index::change_for(&conn, entry)?
    };
    if let index::Change::Unchanged(id) = change {
        return Ok(id);
    }
    let content = index::content_for(entry);
    let conn = core.db()?;
    index::write(&conn, entry, change, content)
}

/// `index::refresh_path` with the same short-lock discipline, for live changes.
fn refresh_unlocked(core: &Arc<Core>, path: &Path) -> crate::error::Result<()> {
    if path.is_dir() {
        let conn = core.db()?;
        return index::refresh_path(&conn, path).or_else(|e| {
            log::warn!("refresh folder {}: {e}", path.display());
            Ok(())
        });
    }
    match index::entry_for(path) {
        Some(entry) => index_entry(core, &entry).map(|_| ()),
        None => {
            let conn = core.db()?;
            index::remove(&conn, path)
        }
    }
}

/// Walks every root, newest files first, and fills the index in small
/// batches so the database lock is never held long.
pub fn start_scan(core: Arc<Core>, app: AppHandle) {
    let spawned = std::thread::Builder::new().name("tote-scan".into()).spawn(move || {
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
        let mut noticed = 0;
        let now = crate::db::now();
        for chunk in entries.chunks(25) {
            for entry in chunk {
                if let Err(error) = index_entry(&core, entry) {
                    log::warn!("index {}: {error}", entry.path.display());
                    continue;
                }
                if now - entry.created <= SCAN_DOWNLOAD_WINDOW_SECS {
                    let checked = core.db().and_then(|conn| index::note_if_downloaded(&conn, &entry.path, SCAN_DOWNLOAD_WINDOW_SECS));
                    match checked {
                        Ok(true) => noticed += 1,
                        Ok(false) => {}
                        Err(error) => log::warn!("download check {}: {error}", entry.path.display()),
                    }
                }
            }
            scanned += chunk.len();
            // Progress every chunk; the row count and the event every 200 files,
            // so the renderer is not asked to refresh on every batch.
            let milestone = scanned % 200 < 25 || scanned == total;
            let files = if milestone { core.db().and_then(|c| index::count(&c)).unwrap_or(0) } else { 0 };
            core.set_status(|s| {
                s.scanned = scanned;
                if milestone {
                    s.files = files;
                }
            });
            if milestone {
                let _ = app.emit(CHANGED_EVENT, ());
            }
            // Yield so her laptop stays responsive while the first scan runs.
            std::thread::sleep(Duration::from_millis(5));
        }
        let seen: std::collections::HashSet<String> = entries.iter().map(|e| e.path.to_string_lossy().into_owned()).collect();
        if let Ok(conn) = core.db() {
            for root in core.roots.all() {
                match index::prune_except(&conn, &root, &seen) {
                    Ok(n) if n > 0 => log::info!("pruned {n} rows under {} (gone, or in skipped folders)", root.display()),
                    Ok(_) => {}
                    Err(error) => log::warn!("prune {}: {error}", root.display()),
                }
            }
        }
        if noticed > 0 {
            log::info!("first scan found {noticed} recent downloads saved outside Downloads or before Tote ran");
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
