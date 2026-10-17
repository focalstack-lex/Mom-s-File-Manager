//! Background work (spec 5.1 and 5.2): the Downloads watcher and sorting
//! pipeline, the index watchers that keep search current, and the initial
//! scan. Everything here runs on its own threads and reports through events.

use crate::error::Result;
use crate::index;
use crate::ops;
use crate::origin;
use crate::paths;
use crate::sorter;
use crate::stability::{self, Pending};
use crate::state::Core;
use crate::tasks;
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
/// How often the To do housekeeping runs (purge, snapshot, morning reminder).
const TASKS_CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
/// The setting that says when Tote was last awake: written at the worker's
/// start, every 15 minutes with the To do check, and at exit (lib.rs). A
/// file in Downloads created after the previous run's value arrived while
/// Tote was closed.
pub const LAST_RUN_KEY: &str = "downloads_last_run";

/// Whether a file in Downloads is a download Tote has not filed, rather than
/// an old file she edited, renamed or put back (audit T-06, T-07): it was
/// created after the last run's marker, or it carries the browser's web
/// stamp and was created within the live window. `run_marker` is the value
/// read when the worker started, so a long download whose part file kept an
/// earlier created time is still new.
pub fn is_new_download(created: i64, stamped: bool, run_marker: i64, now: i64) -> bool {
    created > run_marker || (stamped && now - created <= LIVE_DOWNLOAD_WINDOW_SECS)
}

/// The month folder comes from when the file arrived, not when it was filed
/// (audit T-51): a download at 23:59 on the last day of a month belongs to
/// that month.
fn created_local(created: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(created, 0).single().unwrap_or_else(chrono::Local::now)
}

/// Reads the previous run's marker (now, on the first run, so nothing already
/// in Downloads is touched) and writes this run's.
fn start_run_marker(core: &Core) -> i64 {
    let now = crate::db::now();
    let Ok(conn) = core.db() else { return now };
    let previous = crate::db::get_setting(&conn, LAST_RUN_KEY).ok().flatten().and_then(|value| value.parse::<i64>().ok()).unwrap_or(now);
    if let Err(error) = crate::db::set_setting(&conn, LAST_RUN_KEY, &now.to_string()) {
        log::warn!("run marker not saved: {error}");
    }
    previous
}

/// Writes the run marker as now.
pub fn mark_run(core: &Core) {
    let Ok(conn) = core.db() else { return };
    if let Err(error) = crate::db::set_setting(&conn, LAST_RUN_KEY, &crate::db::now().to_string()) {
        log::warn!("run marker not saved: {error}");
    }
}

/// Downloads that arrived while Tote was closed (audit T-51): every stamped
/// file directly in Downloads created after the previous run's marker,
/// unless an Undo once put it back there (that registry is in memory, and a
/// Windows shutdown skips the exit mark). The watcher only hears live
/// events, so these are fed to it at start.
fn unseen_downloads(core: &Core, run_marker: i64) -> Vec<PathBuf> {
    let Some(downloads) = core.roots.downloads.as_ref() else { return Vec::new() };
    let Ok(read) = std::fs::read_dir(downloads) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in read.flatten() {
        // The ops rows hold the resolved spelling (`confine`), so the sweep
        // compares in that spelling too.
        let path = paths::resolve(&entry.path()).unwrap_or_else(|_| entry.path());
        let Some(file) = index::entry_for(&path) else { continue };
        if file.created <= run_marker || sorter::stays_in_downloads(&sorter::extension(&path)) || origin::read_zone(&path).is_none() {
            continue;
        }
        // A read that fails leaves the file alone: filing needs a reason.
        let put_back = core.db().and_then(|conn| ops::was_put_back(&conn, &path)).unwrap_or(true);
        if put_back {
            log::debug!("{} was put back by an Undo; left in Downloads", path.display());
            continue;
        }
        found.push(path);
    }
    if !found.is_empty() {
        log::info!("{} downloads arrived while Tote was closed", found.len());
    }
    found
}

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
        .map_err(|e| {
            log::warn!("downloads watcher could not be created: {e}");
            crate::error::ToteError::new(crate::error::ErrorCode::Io, "Tote could not start watching Downloads. Restart Tote to try again.")
        })?;
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
        .map_err(|e| {
            log::warn!("index watcher for {} could not be created: {e}", root.display());
            crate::error::ToteError::new(crate::error::ErrorCode::Io, "Tote could not start watching your folders. Restart Tote to try again.")
        })?;
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
    let mut tasks_checked: Option<Instant> = None;
    let roots = core.roots.all();
    let run_marker = start_run_marker(&core);
    let started = Instant::now();
    for path in unseen_downloads(&core, run_marker) {
        if let Ok(meta) = std::fs::metadata(&path) {
            pending.insert(path, Pending::new(meta.len(), started));
        }
    }
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Msg::Candidate(path)) => {
                // The watcher reports the raw root's spelling; Undo noted the
                // resolved one (`confine`), which can differ by more than case
                // when a folder is redirected, so the candidate is resolved first.
                let path = without_stream(path);
                let path = paths::resolve(&path).unwrap_or(path);
                let now = Instant::now();
                if core.placed.covers(&path, now) {
                    // Undo, restore or Undo all just put this file back; it is hers, not a download.
                    log::debug!("download candidate {} was put there by Tote; left alone", path.display());
                    pending.remove(&path);
                    continue;
                }
                log::debug!("download candidate {}", path.display());
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
                // Log lines name the kind, never the file: names often carry
                // a student's name, and the log may be sent for help (audit T-52).
                if let Err(error) = refresh_unlocked(&core, &path) {
                    log::warn!("index refresh of {}: {error}", sorter::describe(&path));
                    log::debug!("index refresh failed for {}", path.display());
                }
                if let Ok(conn) = core.db() {
                    // A browser that saved outside Downloads still stamps the
                    // file; it joins Recent downloads where it is (spec 5.1.3).
                    match index::note_if_downloaded(&conn, &path, LIVE_DOWNLOAD_WINDOW_SECS) {
                        Ok(true) => {
                            log::info!("noticed {} saved outside Downloads", sorter::describe(&path));
                            log::debug!("the download outside Downloads is {}", path.display());
                        }
                        Ok(false) => {}
                        Err(error) => {
                            log::warn!("download check of {}: {error}", sorter::describe(&path));
                            log::debug!("download check failed for {}", path.display());
                        }
                    }
                }
                index_dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        tick(&core, &app, &mut pending, run_marker);
        if index_dirty {
            index_dirty = false;
            let _ = app.emit(CHANGED_EVENT, ());
        }
        if tasks_checked.is_none_or(|at| at.elapsed() >= TASKS_CHECK_EVERY) {
            tasks_checked = Some(Instant::now());
            tasks_check(&core, &app);
            mark_run(&core);
        }
    }
}

/// The To do list's housekeeping: clear old done and deleted tasks, keep the
/// day's snapshot, and show the morning reminder (rules in tasks.rs). Runs
/// when the worker starts and every 15 minutes, so a laptop first opened at
/// 14:00 is reminded then.
fn tasks_check(core: &Arc<Core>, app: &AppHandle) {
    // Settings first: the database mutex is not reentrant.
    let notifications = crate::commands::settings_of(core).notifications;
    let now = chrono::Local::now().naive_local();
    let reminder = {
        let Ok(conn) = core.db() else { return };
        if let Err(error) = tasks::purge(&conn, crate::db::now()) {
            log::warn!("tasks purge: {error}");
        }
        match tasks::snapshot(&conn, &core.data_dir, now.date()) {
            Ok(true) => log::info!("tasks snapshot saved"),
            Ok(false) => {}
            Err(error) => log::warn!("tasks snapshot: {error}"),
        }
        let last = crate::db::get_setting(&conn, tasks::REMINDED_KEY).ok().flatten().and_then(|day| tasks::parse_day(&day));
        match tasks::open_with_due(&conn) {
            Ok(open) => tasks::reminder(now, last, &open, notifications),
            Err(error) => {
                log::warn!("tasks reminder: {error}");
                None
            }
        }
    };
    let Some(reminder) = reminder else { return };
    use tauri_plugin_notification::NotificationExt;
    if let Err(error) = app.notification().builder().title(&reminder.title).body(&reminder.body).show() {
        // Not recorded, so the next check tries again.
        log::warn!("reminder notification: {error}");
        return;
    }
    log::info!("morning reminder shown ({} due today, {} overdue)", reminder.due_today, reminder.overdue);
    if let Ok(conn) = core.db() {
        if let Err(error) = crate::db::set_setting(&conn, tasks::REMINDED_KEY, &now.date().format("%Y-%m-%d").to_string()) {
            log::warn!("reminder day not saved: {error}");
        }
    }
}

fn tick(core: &Arc<Core>, app: &AppHandle, pending: &mut HashMap<PathBuf, Pending>, run_marker: i64) {
    let now = Instant::now();
    let due: Vec<PathBuf> = pending.iter().filter(|(_, p)| now >= p.next_try).map(|(path, _)| path.clone()).collect();
    for path in due {
        let Some(state) = pending.get_mut(&path) else { continue };
        let Ok(meta) = std::fs::metadata(&path) else {
            pending.remove(&path);
            continue;
        };
        state.observe(meta.len(), now);
        // An empty file, or one whose browser is still writing the part file
        // beside it, is not a download yet (audit T-15).
        let in_progress = meta.len() == 0 || stability::has_partial_sibling(&path);
        if state.expired(now) {
            if in_progress {
                // An empty placeholder whose download never finished is nothing to file.
                log::info!("giving up on {} in Downloads: still empty or being downloaded after a day", sorter::describe(&path));
            } else {
                log::info!("giving up on {} in Downloads: still busy after a day", sorter::describe(&path));
                if let Ok(conn) = core.db() {
                    let _ = ops::note_failure(&conn, &path, "The file stayed open in another program for a day, so it was left in Downloads.");
                }
            }
            log::debug!("gave up on {}", path.display());
            pending.remove(&path);
            continue;
        }
        let quiet = state.is_quiet(now);
        let free = quiet && !in_progress && stability::can_open_exclusively(&path);
        if !free {
            state.defer(now);
            log::debug!(
                "waiting on {} ({}), attempt {}, next look in {:?}",
                path.display(),
                if in_progress {
                    "still empty or being downloaded"
                } else if quiet {
                    "another program has it open"
                } else {
                    "still growing"
                },
                state.attempts,
                state.next_try.saturating_duration_since(now)
            );
            continue;
        }
        pending.remove(&path);
        sort_one(core, app, &path, run_marker);
    }
}

/// Files one finished download and tells the renderer. A file that is not a
/// new download (old and unstamped: edited, renamed, or put back by hand)
/// is left where it is.
fn sort_one(core: &Arc<Core>, app: &AppHandle, path: &Path, run_marker: i64) {
    let ext = sorter::extension(path);
    if sorter::stays_in_downloads(&ext) {
        return;
    }
    let Some(entry) = index::entry_for(path) else { return };
    let stamp = origin::read_zone(path);
    if !is_new_download(entry.created, stamp.is_some(), run_marker, crate::db::now()) {
        log::debug!("{} was there before Tote last ran and carries no fresh web stamp; left in Downloads", path.display());
        return;
    }
    let origin = stamp.unwrap_or_else(origin::Origin::unknown);
    let kind = sorter::classify(&ext);
    let school = match core.roots.school_files() {
        Ok(s) => s,
        Err(error) => {
            log::error!("no School Files folder: {error}");
            return;
        }
    };
    let dest = sorter::destination(&school, kind, created_local(entry.created));
    // Settings are read before the database lock is taken: the mutex is not
    // reentrant, and reading them inside the locked block deadlocked the worker.
    let notifications = crate::commands::settings_of(core).notifications;
    let moved = {
        let Ok(conn) = core.db() else { return };
        ops::move_file(&conn, &core.roots, path, &dest, ops::OpKind::AutoSort).inspect(|new_path| {
            if let Err(error) = index::set_origin(&conn, new_path, &origin, crate::db::now()) {
                log::warn!("origin of {} not saved: {error}", kind.words());
                log::debug!("origin not saved for {}", new_path.display());
            }
        })
    };
    match moved {
        Ok(new_path) => {
            let name = new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = format!("School Files > {}", kind.folder_name());
            // The kind, not the name: names often carry a student's name (audit T-52).
            log::info!("sorted {} into {} (from {})", kind.words(), folder, origin.label);
            if notifications {
                notify(app, &name, &folder);
            }
            let _ = app.emit(SORTED_EVENT, Sorted { name, folder, path: new_path.to_string_lossy().into_owned() });
            let _ = app.emit(CHANGED_EVENT, ());
        }
        Err(error) => {
            log::warn!("could not sort {}: {}", kind.words(), error.message);
            log::debug!("could not sort {}", path.display());
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
                    log::warn!("index of {}: {error}", sorter::describe(&entry.path));
                    log::debug!("index failed for {}", entry.path.display());
                    continue;
                }
                if now - entry.created <= SCAN_DOWNLOAD_WINDOW_SECS {
                    let checked = core.db().and_then(|conn| index::note_if_downloaded(&conn, &entry.path, SCAN_DOWNLOAD_WINDOW_SECS));
                    match checked {
                        Ok(true) => noticed += 1,
                        Ok(false) => {}
                        Err(error) => {
                            log::warn!("download check of {}: {error}", sorter::describe(&entry.path));
                            log::debug!("download check failed for {}", entry.path.display());
                        }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The rule that keeps the Downloads promise: only a file that arrived
    /// after Tote last ran, or a freshly stamped one, is a new download. An
    /// old file she edits, renames or puts back is not (audit T-06, T-07).
    #[test]
    fn only_arrivals_and_fresh_stamps_are_new_downloads() {
        let marker = 1_000_000;
        let now = marker + 3_600;
        assert!(is_new_download(marker + 1, false, marker, now), "a file created after the last run, even without a stamp");
        assert!(is_new_download(now, true, marker, now), "a download that just landed");
        assert!(!is_new_download(marker - 86_400, false, marker, now), "an old file edited or renamed stays (T-07)");
        assert!(!is_new_download(marker, false, marker, now), "created at the marker itself is not after it");
        assert!(is_new_download(marker - 60, true, marker, marker + 120), "stamped and created within the live window: a long download that spanned the start");
        assert!(!is_new_download(marker - 60, true, marker, marker + LIVE_DOWNLOAD_WINDOW_SECS + 61), "a stamped file from before the last run, past the window, is hers (an Undo put it back)");
        assert!(is_new_download(marker - 60, true, marker, marker - 60 + LIVE_DOWNLOAD_WINDOW_SECS), "the window's edge still counts");
    }

    #[test]
    fn the_month_folder_is_the_month_the_file_arrived() {
        use chrono::TimeZone;
        let arrived = chrono::Local.with_ymd_and_hms(2026, 10, 31, 23, 59, 0).unwrap();
        let school = Path::new(r"C:\Docs\School Files");
        let dest = sorter::destination(school, sorter::Kind::Pdf, created_local(arrived.timestamp()));
        assert_eq!(dest, PathBuf::from(r"C:\Docs\School Files\PDF\2026-10"), "filed after midnight, still October");
    }

    fn core_with_downloads() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let (desktop, documents, downloads) = (dir.path().join("Desktop"), dir.path().join("Documents"), dir.path().join("Downloads"));
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        let core = Core {
            db: std::sync::Mutex::new(crate::db::open_memory().unwrap()),
            roots: crate::paths::Roots::explicit(desktop, documents, downloads),
            status: std::sync::Mutex::new(Default::default()),
            recovered: false,
            rebuilt: None,
            data_dir: std::path::PathBuf::new(),
            tasks_restored: None,
            tasks_unavailable: false,
            placed: Default::default(),
        };
        (dir, core)
    }

    /// The first run writes the marker as now and reports now, so nothing
    /// already in Downloads counts as new; the next start reads what the
    /// previous run wrote.
    #[test]
    fn the_run_marker_starts_at_now_and_is_read_back() {
        let (_dir, core) = core_with_downloads();
        let before = crate::db::now();
        let first = start_run_marker(&core);
        assert!(first >= before && first <= crate::db::now());
        {
            let conn = core.db().unwrap();
            crate::db::set_setting(&conn, LAST_RUN_KEY, "123456").unwrap();
        }
        assert_eq!(start_run_marker(&core), 123_456, "the previous run's marker");
        mark_run(&core);
        let written: i64 = crate::db::get_setting(&core.db().unwrap(), LAST_RUN_KEY).unwrap().unwrap().parse().unwrap();
        assert!(written >= first);
    }

    /// The start-up sweep (audit T-51): a stamped file that arrived after the
    /// previous run is a candidate; an unstamped file, an installer, a file
    /// from before the marker and a file an Undo put back are not.
    #[cfg(windows)]
    #[test]
    fn the_sweep_finds_stamped_arrivals_and_leaves_the_rest() {
        let (_dir, core) = core_with_downloads();
        let downloads = core.roots.downloads.clone().unwrap();
        let stamp = |path: &Path| std::fs::write(format!("{}:Zone.Identifier", path.to_string_lossy()), "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://www.deped.gov.ph/memo.pdf\r\n").unwrap();
        let arrived = downloads.join("DepEd Memo 123.pdf");
        std::fs::write(&arrived, b"%PDF").unwrap();
        stamp(&arrived);
        let unstamped = downloads.join("Copied from a flash drive.docx");
        std::fs::write(&unstamped, b"copy").unwrap();
        let installer = downloads.join("printer-setup.exe");
        std::fs::write(&installer, b"MZ").unwrap();
        stamp(&installer);
        let put_back = downloads.join("Quiz A.docx");
        std::fs::write(&put_back, b"quiz").unwrap();
        stamp(&put_back);
        let now = crate::db::now();
        {
            let conn = core.db().unwrap();
            let dest = core.roots.documents.clone().unwrap().join("School Files").join("Word");
            ops::move_file(&conn, &core.roots, &put_back, &dest, ops::OpKind::AutoSort).unwrap();
            let row = ops::list(&conn, 1).unwrap().remove(0);
            ops::undo(&conn, &core.roots, &core.placed, row.id).unwrap();
        }
        assert!(put_back.is_file());
        let found = unseen_downloads(&core, now - 60);
        assert_eq!(found, vec![arrived.clone()], "only the stamped arrival that no Undo put back");
        assert!(unseen_downloads(&core, now + 60).is_empty(), "nothing is newer than a marker in the future (the first run)");
    }
}
