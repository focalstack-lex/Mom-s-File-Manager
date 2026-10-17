//! Background work (spec 5.2): the index watchers that keep search current,
//! the initial scan, and the To do check. Everything here runs on its own
//! threads and reports through events. Downloads are not filed on their own
//! any more (Lex's decision, 2026-10-17): she sends a file to Tote from File
//! Explorer's Move to Tote, which filing.rs handles.

use crate::error::Result;
use crate::index;
use crate::sorter;
use crate::state::Core;
use crate::tasks;
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const CHANGED_EVENT: &str = "tote://changed";

/// A stamped file created this recently, anywhere Tote watches, counts as a download.
const LIVE_DOWNLOAD_WINDOW_SECS: i64 = 10 * 60;
/// On the first scan, stamped files from the last month fill Recent downloads.
const SCAN_DOWNLOAD_WINDOW_SECS: i64 = 30 * 86_400;
/// How often the To do housekeeping runs (purge, snapshot, morning reminder).
const TASKS_CHECK_EVERY: Duration = Duration::from_secs(15 * 60);

/// Windows reports a write to `file.mp3:Zone.Identifier` with the stream
/// name attached; the file itself is what matters.
fn without_stream(path: PathBuf) -> PathBuf {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return path };
    match name.split_once(':') {
        Some((file, _stream)) if !file.is_empty() => path.with_file_name(file),
        _ => path,
    }
}

/// Starts the index watchers and the worker thread. The debouncers move
/// into the thread so they live as long as the app.
pub fn start(core: Arc<Core>, app: AppHandle) -> Result<()> {
    let (tx, rx) = mpsc::channel::<PathBuf>();

    let mut debouncers = Vec::new();
    for root in core.roots.all() {
        let tx_idx = tx.clone();
        let mut debouncer = new_debouncer(Duration::from_secs(3), move |result: DebounceEventResult| match result {
            Ok(events) => {
                for event in events {
                    let _ = tx_idx.send(event.path);
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

fn run(core: Arc<Core>, app: AppHandle, rx: mpsc::Receiver<PathBuf>) {
    let mut index_dirty = false;
    let mut tasks_checked: Option<Instant> = None;
    let roots = core.roots.all();
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(path) => {
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
                    // A stamped file joins Recent downloads where the browser
                    // saved it (spec 5.1.3); it stays there until she sends it
                    // to Tote.
                    match index::note_if_downloaded(&conn, &path, LIVE_DOWNLOAD_WINDOW_SECS) {
                        Ok(true) => {
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
        if index_dirty {
            index_dirty = false;
            let _ = app.emit(CHANGED_EVENT, ());
        }
        if tasks_checked.is_none_or(|at| at.elapsed() >= TASKS_CHECK_EVERY) {
            tasks_checked = Some(Instant::now());
            tasks_check(&core, &app);
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
}
