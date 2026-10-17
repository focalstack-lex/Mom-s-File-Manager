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

/// Whether an Activity row says Tote put this file in Downloads for her
/// after it was created (Undo, restore, Undo all, or a Move to Downloads),
/// so it is hers and is not filed (audit T-06). The in-memory registry
/// covers the first ten minutes after the landing; this check holds for as
/// long as the row does, across a restart too. A read that fails leaves the
/// file alone: filing needs a reason.
fn put_there_for_her(conn: &rusqlite::Connection, path: &Path, created: i64) -> bool {
    match ops::was_put_back(conn, path, created) {
        Ok(false) => false,
        Ok(true) => {
            log::debug!("{} was put in Downloads by an Undo or a Move; left there", path.display());
            true
        }
        Err(error) => {
            log::warn!("could not check Activity before filing {}: {}", sorter::describe(path), error.message);
            log::debug!("the Activity check failed for {}", path.display());
            true
        }
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
        if core.db().map(|conn| put_there_for_her(&conn, &path, file.created)).unwrap_or(true) {
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
                            // Downloads is a root too, so this also fires for a download
                            // the sorter is about to file; the line says what is true now.
                            log::info!("noticed {} with a web stamp; it joins Recent downloads where it is", sorter::describe(&path));
                            log::debug!("the stamped file is {}", path.display());
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
            give_up(core, &path, in_progress, run_marker);
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

/// A file still busy after a day is dropped from the watch. Only one Tote
/// would have filed gets a failure row in Activity; an old file she keeps
/// open in Word, an empty placeholder whose download never finished, or a
/// file an Undo or a Move put in Downloads for her (the guard `sort_one`
/// applies) is not hers to hear about (audit T-06, T-07, T-15).
fn give_up(core: &Core, path: &Path, in_progress: bool, run_marker: i64) {
    if in_progress {
        log::info!("giving up on {} in Downloads: still empty or being downloaded after a day", sorter::describe(path));
    } else if let Some((entry, _)) = new_download_at(path, run_marker) {
        let Ok(conn) = core.db() else { return };
        if put_there_for_her(&conn, path, entry.created) {
            return;
        }
        log::info!("giving up on {} in Downloads: still busy after a day", sorter::describe(path));
        if let Err(error) = ops::note_failure(&conn, path, "The file stayed open in another program for a day, so it was left in Downloads.") {
            log::warn!("the row for {} left in Downloads was not saved: {}", sorter::describe(path), error.message);
        }
    } else {
        log::debug!("gave up waiting on {}, which Tote would not have filed; no row", path.display());
        return;
    }
    log::debug!("gave up on {}", path.display());
}

/// The first look at a candidate: a file Tote never files (an installer or
/// script, a file it cannot read, or one that is not a new download: old
/// and unstamped, so edited, renamed or put back by hand) is None;
/// otherwise its entry and web stamp.
fn new_download_at(path: &Path, run_marker: i64) -> Option<(index::Entry, Option<origin::Origin>)> {
    if sorter::stays_in_downloads(&sorter::extension(path)) {
        return None;
    }
    let entry = index::entry_for(path)?;
    let stamp = origin::read_zone(path);
    if !is_new_download(entry.created, stamp.is_some(), run_marker, crate::db::now()) {
        log::debug!("{} was there before Tote last ran and carries no fresh web stamp; left in Downloads", path.display());
        return None;
    }
    Some((entry, stamp))
}

/// Files one finished download and tells the renderer. A file that is not a
/// new download, or one an Activity row says Tote put in Downloads for her,
/// is left where it is.
fn sort_one(core: &Arc<Core>, app: &AppHandle, path: &Path, run_marker: i64) {
    let Some((entry, stamp)) = new_download_at(path, run_marker) else { return };
    let origin = stamp.unwrap_or_else(origin::Origin::unknown);
    let kind = sorter::classify(&sorter::extension(path));
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
    // The registry forgets a path after ten minutes and a restart empties
    // it; the Activity rows still say whether she put the file there. Asked
    // right before the move, which then takes the lock itself, to record
    // and to finish, and moves the file with no lock held, so a search
    // answers while a download is copied to School Files on another drive
    // (audit T-12).
    match core.db() {
        Ok(conn) if !put_there_for_her(&conn, path, entry.created) => {}
        _ => return,
    }
    let moved = ops::move_file(&**core, &core.roots, &core.placed, path, &dest, ops::OpKind::AutoSort).inspect(|new_path| {
        if let Err(error) = core.db().and_then(|conn| index::set_origin(&conn, new_path, &origin, crate::db::now())) {
            log::warn!("origin of {} not saved: {error}", kind.words());
            log::debug!("origin not saved for {}", new_path.display());
        }
    });
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

/// `index::refresh_path` for live changes, with the database lock held only
/// for short lookups and writes: the slow parts (walking a folder, reading
/// the words inside each file) run with no lock, so a search never waits
/// on a PDF or on a folder copied in. The 18,000-file scan on Lex's machine
/// kept the Find screen on "Searching" because the old upsert held the lock
/// across extraction (2026-10-12), and a folder that appeared in a root was
/// walked and read whole under one lock until 2026-10-17 (audit T-12): it
/// now goes in chunk by chunk through the scan's own routine.
fn refresh_unlocked(core: &Arc<Core>, path: &Path) -> crate::error::Result<()> {
    if path.is_dir() {
        let Some(entries) = index::walk_folder(path) else {
            // A folder the scanner skips (build output, a hidden folder) keeps nothing in the index.
            return index::remove(&*core.db()?, path);
        };
        let now = crate::db::now();
        for (at, chunk) in entries.chunks(SCAN_CHUNK).enumerate() {
            if at > 0 {
                // A search that waited gets its turn before the next chunk.
                std::thread::sleep(Duration::from_millis(5));
            }
            scan_chunk(core, chunk, now, LIVE_DOWNLOAD_WINDOW_SECS);
        }
        return Ok(());
    }
    index::refresh_file(&**core, path)
}

/// Files per scan chunk: one lookup under the lock, extraction with no lock
/// held, one transaction for the writes (audit T-11).
const SCAN_CHUNK: usize = 25;

/// What one scan chunk did: the files it wrote to the index and the recent
/// downloads it noticed outside Downloads.
#[derive(Default)]
struct Chunked {
    written: usize,
    noticed: usize,
}

/// Indexes one chunk of the scan, or of a folder that appeared while Tote
/// runs: a short lock to learn which files changed, extraction with no lock
/// held, then one lock for the chunk's transaction and the download check,
/// which marks a stamped file created within `window` seconds of `now`.
fn scan_chunk(core: &Arc<Core>, chunk: &[index::Entry], now: i64, window: i64) -> Chunked {
    let changes: Vec<Option<index::Change>> = match core.db() {
        Ok(conn) => chunk
            .iter()
            .map(|entry| match index::change_for(&conn, entry) {
                Ok(change) => Some(change),
                Err(error) => {
                    log::warn!("index of {}: {error}", sorter::describe(&entry.path));
                    log::debug!("index failed for {}", entry.path.display());
                    None
                }
            })
            .collect(),
        Err(error) => {
            log::warn!("scan: {error}");
            return Chunked::default();
        }
    };
    let prepared: Vec<index::Prepared<'_>> = chunk
        .iter()
        .zip(changes)
        .filter_map(|(entry, change)| match change? {
            index::Change::Unchanged(_) => None,
            change => Some(index::Prepared { entry, change, content: index::content_for(entry) }),
        })
        .collect();
    let conn = match core.db() {
        Ok(conn) => conn,
        Err(error) => {
            log::warn!("scan: {error}");
            return Chunked::default();
        }
    };
    let mut done = Chunked::default();
    if !prepared.is_empty() {
        match index::write_batch(&conn, prepared) {
            Ok(written) => done.written = written,
            Err(error) => log::warn!("index of a chunk of {} files: {error}", chunk.len()),
        }
    }
    for entry in chunk.iter().filter(|entry| now - entry.created <= window) {
        match index::note_if_downloaded(&conn, &entry.path, window) {
            Ok(true) => done.noticed += 1,
            Ok(false) => {}
            Err(error) => {
                log::warn!("download check of {}: {error}", sorter::describe(&entry.path));
                log::debug!("download check failed for {}", entry.path.display());
            }
        }
    }
    done
}

/// A scan that wrote at least this many files (the first scan, a big folder
/// added) is followed by the merge; after a smaller one FTS5's own merging
/// is enough (audit T-11).
const MERGE_AFTER_FILES: usize = 500;
/// The most merge steps one scan runs, so the loop ends whatever the
/// index says: at `index::MERGE_STEP_PAGES` a step, far more index than
/// her laptop holds.
const MERGE_MAX_STEPS: usize = 4_096;
/// A merge step that held the lock this long is worth a warning: a search
/// may have waited for it.
const MERGE_STEP_WARN: Duration = Duration::from_secs(2);

/// Merges the search index in bounded steps (`index::merge_step`), taking
/// and giving back the database lock for each, so a search between two
/// steps waits for one step at most; the one-shot 'optimize' it replaces
/// rewrote the whole index under one lock at the moment the walkthrough
/// says to type (audit T-11). Returns the steps run and the longest hold.
fn merge_index(core: &Core) -> (usize, Duration) {
    let started = Instant::now();
    let mut steps = 0;
    let mut longest = Duration::ZERO;
    for step in 0..MERGE_MAX_STEPS {
        let outcome = core.db().and_then(|conn| {
            let held = Instant::now();
            index::merge_step(&conn, step == 0).map(|more| (more, held.elapsed()))
        });
        let more = match outcome {
            Ok((more, held)) => {
                steps += 1;
                longest = longest.max(held);
                more
            }
            Err(error) => {
                log::warn!("search index merge stopped at step {}: {error}", step + 1);
                break;
            }
        };
        if !more {
            break;
        }
        // A search that waited gets its turn before the next step.
        std::thread::sleep(Duration::from_millis(5));
    }
    let (total, hold) = (started.elapsed().as_millis(), longest.as_millis());
    if longest >= MERGE_STEP_WARN {
        log::warn!("search index merged in {steps} steps, {total} ms; the longest step held the lock {hold} ms");
    } else {
        log::info!("search index merged in {steps} steps, {total} ms (longest step {hold} ms)");
    }
    (steps, longest)
}

/// The full integrity check reads the whole file, so it runs here, once the
/// window is up and the scan is done, on its own connection: the lock stays
/// free for searches. Damage leaves a marker that makes the next start
/// rebuild (audit T-50).
fn full_check(core: &Core) {
    match crate::db::full_check(&core.data_dir) {
        Ok(true) => log::debug!("database: the full check passed"),
        Ok(false) => {
            log::error!("database: the full check found damage; the next start rebuilds the index");
            if let Err(error) = core.db().and_then(|conn| crate::db::mark_check_failed(&conn)) {
                log::warn!("database: the failed check could not be recorded: {error}");
            }
        }
        Err(error) => log::warn!("database: the full check could not run: {error}"),
    }
}

/// Walks every root, newest files first, and fills the index chunk by
/// chunk (`scan_chunk`) so the database lock is never held long, merges
/// the search index in steps when a big scan is done, then runs the full
/// integrity check.
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
        let mut written = 0;
        let now = crate::db::now();
        for chunk in entries.chunks(SCAN_CHUNK) {
            let chunked = scan_chunk(&core, chunk, now, SCAN_DOWNLOAD_WINDOW_SECS);
            noticed += chunked.noticed;
            written += chunked.written;
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
        // The chunks left the search index in segments. After a scan that
        // wrote many files they are merged here in bounded steps, the lock
        // given back between them; after a smaller scan FTS5 merges on its
        // own as files are written (audit T-11).
        if written >= MERGE_AFTER_FILES {
            merge_index(&core);
        } else {
            log::debug!("search index: {written} files written this scan, no merge needed");
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
        full_check(&core);
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

    /// The merge after a big scan runs step by step under separate locks
    /// and ends on its own; the lock is free after it (audit T-11).
    #[test]
    fn the_merge_after_a_scan_runs_in_steps_and_ends() {
        let (_dir, core) = core_with_downloads();
        {
            let conn = core.db().unwrap();
            conn.execute_batch("INSERT INTO files_fts(files_fts, rank) VALUES('automerge', 0)").unwrap();
            let entries: Vec<index::Entry> = (0..MERGE_AFTER_FILES as i64)
                .map(|i| index::Entry { path: PathBuf::from(format!(r"C:\Lab\Quiz {i}.docx")), size: 10, modified: i, created: i, content_available: true })
                .collect();
            for chunk in entries.chunks(SCAN_CHUNK) {
                let prepared = chunk.iter().map(|e| index::Prepared { entry: e, change: index::Change::New, content: Some(format!("Santos attendance {}", e.modified)) }).collect();
                assert_eq!(index::write_batch(&conn, prepared).unwrap(), chunk.len());
            }
        }
        let (steps, longest) = merge_index(&core);
        assert!((2..MERGE_MAX_STEPS).contains(&steps), "the segments the chunks left take a first step and at least the one that says it is done ({steps})");
        assert!(longest < MERGE_STEP_WARN, "a step on a small index is quick");
        let conn = core.db().unwrap();
        assert_eq!(index::search(&conn, "santos", &index::Filters::default(), 10).unwrap().len(), 10, "the lock is free and the index answers");
        assert!(!index::merge_step(&conn, false).unwrap(), "nothing was left to merge");
    }

    /// A folder that appears in a root (copied in, or moved from outside
    /// Tote's folders) is walked and read with no lock held and written
    /// chunk by chunk, the scan's own routine (audit T-12): every file in it
    /// is found, by name and by its words; a folder the scanner skips keeps
    /// nothing in the index.
    #[test]
    fn a_folder_moved_in_is_indexed_chunk_by_chunk() {
        let (_dir, core) = core_with_downloads();
        let core = Arc::new(core);
        let folder = core.roots.documents.clone().unwrap().join("Grade 7 Quizzes");
        std::fs::create_dir_all(folder.join("Week 2")).unwrap();
        let files = SCAN_CHUNK * 2 + 3;
        for i in 0..files {
            let inside = if i % 2 == 0 { folder.clone() } else { folder.join("Week 2") };
            std::fs::write(inside.join(format!("Quiz {i}.txt")), format!("Santos attendance {i}")).unwrap();
        }
        refresh_unlocked(&core, &folder).unwrap();
        let conn = core.db().unwrap();
        assert_eq!(index::count(&conn).unwrap(), files);
        assert_eq!(index::search(&conn, "santos", &index::Filters::default(), 100).unwrap().len(), files, "every file's words are in");
        assert_eq!(index::search(&conn, "quiz 52", &index::Filters::default(), 10).unwrap().len(), 1, "and its name");
        drop(conn);
        let skipped = core.roots.documents.clone().unwrap().join("node_modules");
        std::fs::create_dir_all(&skipped).unwrap();
        std::fs::write(skipped.join("index.txt"), "Santos").unwrap();
        refresh_unlocked(&core, &skipped).unwrap();
        assert_eq!(index::count(&core.db().unwrap()).unwrap(), files, "nothing under a skipped folder");
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
            ops::move_file(&conn, &core.roots, &core.placed, &put_back, &dest, ops::OpKind::AutoSort).unwrap();
            let row = ops::list(&conn, 1).unwrap().remove(0);
            ops::undo(&conn, &core.roots, &core.placed, row.id).unwrap();
        }
        assert!(put_back.is_file());
        let found = unseen_downloads(&core, now - 60);
        assert_eq!(found, vec![arrived.clone()], "only the stamped arrival that no Undo put back");
        assert!(unseen_downloads(&core, now + 60).is_empty(), "nothing is newer than a marker in the future (the first run)");
    }

    /// The live path's guard after an Undo (audit T-06): the registry alone
    /// forgets the path after ten minutes, so `sort_one` also asks the
    /// Activity rows, which know the file was put back for as long as the
    /// row lives; a later download at the same path is still filed.
    #[test]
    fn a_put_back_file_is_left_alone_after_the_registry_forgets_it() {
        use crate::state::PLACED_FOR;
        let (_dir, core) = core_with_downloads();
        let src = core.roots.downloads.clone().unwrap().join("Quiz A.docx");
        std::fs::write(&src, b"quiz").unwrap();
        let created = index::entry_for(&src).unwrap().created;
        let dest = core.roots.documents.clone().unwrap().join("School Files").join("Word");
        let row = {
            let conn = core.db().unwrap();
            ops::move_file(&conn, &core.roots, &core.placed, &src, &dest, ops::OpKind::AutoSort).unwrap();
            let row = ops::list(&conn, 1).unwrap().remove(0);
            ops::undo(&conn, &core.roots, &core.placed, row.id).unwrap();
            row
        };
        assert!(src.is_file(), "Undo put it back");
        // What sort_one reads after the put-back: on the same drive the move kept the created time.
        let back = index::entry_for(&src).unwrap().created;
        assert_eq!(back, created, "a same-drive move keeps the created time");
        let now = Instant::now();
        assert!(core.placed.covers(&src, now), "the registry covers it at first");
        assert!(!core.placed.covers(&src, now + PLACED_FOR), "and forgets it after its time: the registry alone is not the guard");
        let conn = core.db().unwrap();
        let undone_at: i64 = conn.query_row("SELECT undone_at FROM ops WHERE id = ?1", [row.id], |r| r.get(0)).unwrap();
        assert!(undone_at >= row.created_at, "the row recorded the moment of the Undo");
        assert!(put_there_for_her(&conn, &src, back), "the Activity row still says she put it back");
        assert!(!put_there_for_her(&conn, &src, undone_at + 1), "a download that lands at the same path after the Undo is newer than it and is filed");
        assert!(!put_there_for_her(&conn, &core.roots.downloads.clone().unwrap().join("Quiz B.docx"), back), "another path is not covered");
    }

    /// Giving up on a busy file writes a failure row only when Tote would
    /// have filed it: an old file she keeps open all day gets no row, and
    /// neither does an empty placeholder or a file an Undo put back for her
    /// (audit T-06, T-07, T-15).
    #[test]
    fn giving_up_writes_a_failure_row_only_for_a_new_download() {
        let (_dir, core) = core_with_downloads();
        let downloads = core.roots.downloads.clone().unwrap();
        let old = downloads.join("Old Lesson.docx");
        std::fs::write(&old, b"old").unwrap();
        let now = crate::db::now();
        // A marker in the future stands for a file that was there before Tote last ran.
        give_up(&core, &old, false, now + 60);
        assert!(ops::list(&core.db().unwrap(), 10).unwrap().is_empty(), "an old file she kept open gets no row");
        give_up(&core, &old, true, now - 60);
        assert!(ops::list(&core.db().unwrap(), 10).unwrap().is_empty(), "an empty placeholder gets no row");
        let installer = downloads.join("printer-setup.exe");
        std::fs::write(&installer, b"MZ").unwrap();
        give_up(&core, &installer, false, now - 60);
        assert!(ops::list(&core.db().unwrap(), 10).unwrap().is_empty(), "an installer stays in Downloads anyway, so no row");
        give_up(&core, &old, false, now - 60);
        let rows = ops::list(&core.db().unwrap(), 10).unwrap();
        assert_eq!(rows.len(), 1, "a new download still busy after a day is reported");
        assert_eq!((rows[0].kind.as_str(), rows[0].status.as_str()), ("autoSort", "failed"));
        assert_eq!(rows[0].message.as_deref(), Some("The file stayed open in another program for a day, so it was left in Downloads."));
        // A file Undo put back in Downloads, created in this run, that she then keeps open all day:
        // Tote was never going to file it, so the undone filing stays the only row about it (audit T-06).
        let put_back = downloads.join("Quiz A.docx");
        std::fs::write(&put_back, b"quiz").unwrap();
        let created = index::entry_for(&put_back).unwrap().created;
        {
            let conn = core.db().unwrap();
            let dest = core.roots.documents.clone().unwrap().join("School Files").join("Word");
            ops::move_file(&conn, &core.roots, &core.placed, &put_back, &dest, ops::OpKind::AutoSort).unwrap();
            let row = ops::list(&conn, 1).unwrap().remove(0);
            ops::undo(&conn, &core.roots, &core.placed, row.id).unwrap();
        }
        give_up(&core, &put_back, false, created - 1);
        let wanted = put_back.to_string_lossy().into_owned();
        let rows = ops::list(&core.db().unwrap(), 10).unwrap();
        let about_it: Vec<&ops::OpRow> = rows.iter().filter(|r| r.src == wanted).collect();
        assert_eq!(about_it.len(), 1, "only the undone filing names it: no failure row");
        assert_eq!(about_it[0].status, "undone");
    }
}
