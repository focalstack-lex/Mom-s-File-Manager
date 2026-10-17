//! The search index (spec 5.2 and 5.3): what is on disk, what is inside it,
//! and the ranked, grouped answers to "where is that file".

use crate::db;
use crate::dupes;
use crate::error::{Result, ToteError};
use crate::extract;
use crate::origin::Origin;
use crate::sorter;
use crate::state::Db;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub parent: String,
    pub name: String,
    pub ext: String,
    pub kind: String,
    pub size: u64,
    pub modified: i64,
    pub origin_label: Option<String>,
    pub downloaded_at: Option<i64>,
    pub group_key: String,
    pub snippet: Option<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub latest: FileRow,
    pub others: Vec<FileRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    pub from: Option<i64>,
    pub to: Option<i64>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub scanning: bool,
    pub scanned: usize,
    pub total: usize,
    pub files: usize,
}

/// What the scanner learns about one file before touching the database.
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: i64,
    /// When the file was created on this computer; a download's arrival time.
    pub created: i64,
    pub content_available: bool,
}

const SKIP_DIRS: &[&str] = &["AppData", "node_modules", ".git", "$RECYCLE.BIN", "System Volume Information", "__pycache__"];

/// Build output folders, skipped only when they sit next to a project file,
/// so a teacher's own folder that happens to be called "build" or "out" stays.
const BUILD_DIRS: &[&str] = &["target", "dist", "build", "out", "bin", "obj"];
const PROJECT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "CMakeLists.txt",
];
const PROJECT_EXTENSIONS: &[&str] = &["csproj", "sln", "vcxproj", "fsproj"];

fn is_project_dir(dir: &Path) -> bool {
    if PROJECT_MARKERS.iter().any(|m| dir.join(m).is_file()) {
        return true;
    }
    std::fs::read_dir(dir)
        .map(|read| read.flatten().any(|e| PROJECT_EXTENSIONS.contains(&sorter::extension(&e.path()).as_str())))
        .unwrap_or(false)
}

/// Name-based skip rules (no attribute check), safe to apply to ancestors.
fn skipped_by_name(dir: &Path) -> bool {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
        return true;
    }
    if name == "venv" && dir.join("pyvenv.cfg").is_file() {
        return true;
    }
    BUILD_DIRS.contains(&name.as_str()) && dir.parent().map(is_project_dir).unwrap_or(false)
}

/// True when a folder between the watched root and `path` is one the scanner
/// skips, so live change events from build output do not sneak back into the
/// index. Folders above the root never count: a root may itself live under
/// AppData (test mode) or a hidden folder.
pub fn under_skipped_dir(path: &Path, roots: &[PathBuf]) -> bool {
    let Some(root) = roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.components().count()) else {
        return false;
    };
    path.ancestors().skip(1).take_while(|a| a != root).any(skipped_by_name)
}

/// Metadata for a path, or None when it should not be indexed.
pub fn entry_for(path: &Path) -> Option<Entry> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || is_hidden_or_system(&meta) {
        return None;
    }
    let name = path.file_name()?.to_string_lossy();
    if name.starts_with('.') || name.starts_with("~$") {
        return None;
    }
    let ext = sorter::extension(path);
    if matches!(ext.as_str(), "crdownload" | "part" | "tmp" | "download") {
        return None;
    }
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let created = meta
        .created()
        .ok()
        .and_then(|c| c.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(modified);
    Some(Entry { path: path.to_path_buf(), size: meta.len(), modified, created, content_available: content_present(&meta) })
}

#[cfg(windows)]
fn is_hidden_or_system(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden_or_system(_meta: &std::fs::Metadata) -> bool {
    false
}

/// OneDrive "files on demand" placeholders must not be downloaded by indexing.
#[cfg(windows)]
fn content_present(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    const RECALL_ON_OPEN: u32 = 0x0004_0000;
    meta.file_attributes() & (OFFLINE | RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN) == 0
}

#[cfg(not(windows))]
fn content_present(_meta: &std::fs::Metadata) -> bool {
    true
}

fn should_skip_dir(path: &Path) -> bool {
    if skipped_by_name(path) {
        return true;
    }
    std::fs::metadata(path).map(|m| is_hidden_or_system(&m)).unwrap_or(true)
}

/// Every indexable file under `root`, recursively.
pub fn walk(root: &Path, out: &mut Vec<Entry>) {
    let Ok(read) = std::fs::read_dir(root) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if !should_skip_dir(&path) {
                walk(&path, out);
            }
        } else if file_type.is_file() {
            if let Some(e) = entry_for(&path) {
                out.push(e);
            }
        }
    }
}

/// What a folder that appeared or changed holds, for the watcher: None when
/// the scanner skips that folder (its rows are then removed), else every
/// indexable file under it. It only reads the disk, so it runs with no lock
/// held; the files then go in chunk by chunk the way the scan writes them
/// (audit T-12).
pub fn walk_folder(path: &Path) -> Option<Vec<Entry>> {
    if should_skip_dir(path) {
        return None;
    }
    let mut entries = Vec::new();
    walk(path, &mut entries);
    Some(entries)
}

/// What the index knows about a file before it is written: nothing, a row
/// that still matches the disk, or a row that needs refreshing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Unchanged(i64),
    New,
    Modified(i64),
}

/// A quick lookup, meant to run under a short lock. Text extraction, the
/// slow part, happens outside the lock between `change_for` and `write`.
pub fn change_for(conn: &Connection, entry: &Entry) -> Result<Change> {
    let path_text = entry.path.to_string_lossy().into_owned();
    let existing: Option<(i64, i64, i64)> = conn
        .query_row("SELECT id, modified, size FROM files WHERE path = ?1", [&path_text], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(match existing {
        Some((id, modified, size)) if modified == entry.modified && size as u64 == entry.size => Change::Unchanged(id),
        Some((id, _, _)) => Change::Modified(id),
        None => Change::New,
    })
}

/// The slow part, with no lock held: reads the words inside the file.
pub fn content_for(entry: &Entry) -> Option<String> {
    if !entry.content_available {
        return None;
    }
    extract::extract_text(&entry.path, &sorter::extension(&entry.path), entry.size)
}

/// Inserts or refreshes one file in one go. Convenient where the lock is
/// already held briefly (tests, undo); the scanner and watcher use the three
/// steps separately so extraction never blocks searches.
pub fn upsert(conn: &Connection, entry: &Entry) -> Result<i64> {
    match change_for(conn, entry)? {
        Change::Unchanged(id) => Ok(id),
        change => write(conn, entry, change, content_for(entry)),
    }
}

/// A write that fails says not saved; `From<rusqlite::Error>` assumes a read.
fn run(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
    conn.execute(sql, params).map_err(ToteError::db_write)
}

/// Writes a new or changed row and its searchable text, together or not at
/// all. Returns the row id. A savepoint rather than a transaction, so it
/// works inside a caller's transaction too; the scan's chunks go through
/// `write_batch`, which leaves the savepoint out on purpose.
pub fn write(conn: &Connection, entry: &Entry, change: Change, content: Option<String>) -> Result<i64> {
    write_then(conn, entry, change, content, |_| Ok(()))
}

/// `write` with one more step between the row and the release; a test
/// makes the release itself fail through it.
fn write_then(conn: &Connection, entry: &Entry, change: Change, content: Option<String>, before_release: impl FnOnce(&Connection) -> Result<()>) -> Result<i64> {
    if let Change::Unchanged(id) = change {
        return Ok(id);
    }
    atomically(conn, "index_write", |conn| write_inner(conn, entry, change, content.as_deref()).and_then(|id| before_release(conn).map(|()| id)))
}

/// Runs `work` inside the savepoint `name`: what it wrote stays together or
/// none of it does (audit T-48). A savepoint rather than a transaction, so
/// it nests in a caller's transaction too.
fn atomically<T>(conn: &Connection, name: &str, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    run(conn, &format!("SAVEPOINT {name}"), [])?;
    let done = work(conn).and_then(|value| run(conn, &format!("RELEASE {name}"), []).map(|_| value));
    if done.is_err() {
        // The release can fail too (a full disk, a constraint checked at
        // commit), and a savepoint left open would make every later
        // transaction fail at BEGIN and swallow every later write until
        // exit, so the undo runs on any failure. SQLite rolls back on its
        // own after some errors; the savepoint is then already gone and the
        // undo says so, which is no failure.
        if let Err(undo) = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}")) {
            if conn.is_autocommit() {
                log::debug!("index: the failed {name} was already rolled back: {undo}");
            } else {
                log::warn!("index: a half-done {name} could not be undone: {undo}");
            }
        }
    }
    done
}

/// One scanned file with what the lookup and the extraction found, ready
/// for `write_batch`.
pub struct Prepared<'a> {
    pub entry: &'a Entry,
    pub change: Change,
    pub content: Option<String>,
}

/// Writes a chunk of scanned files in one transaction, so the scan commits
/// once per chunk instead of once per file and FTS5 flushes the chunk's
/// words into one segment instead of one per file (audit T-11). The files
/// go in without `write`'s savepoint: FTS5 flushes a segment at every
/// savepoint too (its xSavepoint method), which left one segment per file
/// however the chunk was committed. A file whose write fails costs only
/// itself: the chunk is rolled back, the file is logged and left out (the
/// watcher's next event or the next scan picks it up; writing it again
/// here could loop), and the rest goes in on the next pass with their
/// lookups refreshed, since the usual cause is the watcher indexing files
/// of the chunk meanwhile (a New row that is suddenly there). Returns how
/// many files were written.
pub fn write_batch(conn: &Connection, prepared: Vec<Prepared<'_>>) -> Result<usize> {
    let mut files: Vec<Prepared<'_>> = prepared.into_iter().filter(|file| !matches!(file.change, Change::Unchanged(_))).collect();
    loop {
        let tx = conn.unchecked_transaction().map_err(ToteError::db_write)?;
        let mut failed = None;
        for (at, file) in files.iter().enumerate() {
            if let Err(error) = write_inner(&tx, file.entry, file.change, file.content.as_deref()) {
                log::warn!("index of {}: {error}", sorter::describe(&file.entry.path));
                log::debug!("index failed for {}", file.entry.path.display());
                failed = Some(at);
                break;
            }
        }
        match failed {
            Some(at) => {
                // SQLite rolls back on its own after some errors; then
                // there is nothing left to roll back, which is fine.
                if let Err(error) = tx.rollback() {
                    log::debug!("index: the chunk was already rolled back: {error}");
                }
                files.remove(at);
                files.retain_mut(|file| match change_for(conn, file.entry) {
                    Ok(Change::Unchanged(_)) => false,
                    Ok(change) => {
                        file.change = change;
                        true
                    }
                    Err(error) => {
                        log::warn!("index of {}: {error}", sorter::describe(&file.entry.path));
                        false
                    }
                });
            }
            None => {
                tx.commit().map_err(ToteError::db_write)?;
                return Ok(files.len());
            }
        }
    }
}

/// Leaf pages one merge step writes before it gives the lock back: about a
/// quarter of a megabyte at FTS5's page size. A step of 256 pages held the
/// lock for 3.9 s on a slow drive in the fix-up probe of 2026-10-17 (a
/// 20,000-file index on E:), so the step is a quarter of that and a search
/// between two steps waits about a second there, less on a faster disk
/// (audit T-11).
pub const MERGE_STEP_PAGES: i64 = 64;

/// One step of the merge that FTS5's 'optimize' does in one go, bounded to
/// `MERGE_STEP_PAGES`; the scan runs the steps under separate locks. The
/// first step of a run gathers every segment (a negative count) and the
/// later steps carry that merge on (a positive count), as the FTS5
/// documentation prescribes, so segments a watcher adds meanwhile do not
/// restart it. Returns whether the step found segments to merge, read the
/// way the documentation says: total_changes() grows by two or more when
/// work was done and by less when there was none, so a loop ends here.
pub fn merge_step(conn: &Connection, first: bool) -> Result<bool> {
    let before = conn.total_changes();
    let pages = if first { -MERGE_STEP_PAGES } else { MERGE_STEP_PAGES };
    run(conn, "INSERT INTO files_fts(files_fts, rank) VALUES('merge', ?1)", [pages])?;
    Ok(conn.total_changes() - before >= 2)
}

fn write_inner(conn: &Connection, entry: &Entry, change: Change, content: Option<&str>) -> Result<i64> {
    let path_text = entry.path.to_string_lossy().into_owned();
    let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = entry.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = sorter::extension(&entry.path);
    let kind = sorter::classify(&ext).as_str();
    let parent = entry.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let group_key = dupes::group_key(&stem, &ext);

    let id = match change {
        Change::Unchanged(id) => return Ok(id),
        Change::Modified(id) => {
            run(
                conn,
                "UPDATE files SET size = ?2, modified = ?3, content_indexed = ?4 WHERE id = ?1",
                params![id, entry.size as i64, entry.modified, content.is_some() as i64],
            )?;
            run(conn, "DELETE FROM files_fts WHERE rowid = ?1", [id])?;
            id
        }
        Change::New => {
            run(
                conn,
                "INSERT INTO files(path, parent, name, ext, kind, size, modified, group_key, content_indexed) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![path_text, parent, name, ext, kind, entry.size as i64, entry.modified, group_key, content.is_some() as i64],
            )?;
            conn.last_insert_rowid()
        }
    };
    // A document that happens to hold the snippet marks must not light up.
    let content = match content {
        Some(text) if text.contains([MARK_START, MARK_END]) => std::borrow::Cow::Owned(text.replace([MARK_START, MARK_END], " ")),
        Some(text) => std::borrow::Cow::Borrowed(text),
        None => std::borrow::Cow::Borrowed(""),
    };
    run(
        conn,
        "INSERT INTO files_fts(rowid, name, content) VALUES(?1, ?2, ?3)",
        params![id, indexed_name(&name), content.as_ref()],
    )?;
    Ok(id)
}

/// Indexes a path from disk if it is indexable, otherwise removes any stale row.
pub fn refresh_path(conn: &Connection, path: &Path) -> Result<()> {
    if path.is_dir() && should_skip_dir(path) {
        return remove(conn, path);
    }
    match entry_for(path) {
        Some(entry) => upsert(conn, &entry).map(|_| ()),
        None => {
            if path.is_dir() {
                let mut entries = Vec::new();
                walk(path, &mut entries);
                for entry in &entries {
                    upsert(conn, entry)?;
                }
                Ok(())
            } else {
                remove(conn, path)
            }
        }
    }
}

/// `refresh_path` for one file with the scan's discipline, for a caller
/// that gives the lock back between steps (a moved file the index did not
/// know yet, a file Undo brought back, a file the watcher saw change): a
/// short hold to look the file up, its words read with no lock held, a
/// short hold to write it. The lookup is asked again at the write, so a row
/// written meanwhile is refreshed rather than refused as taken.
pub fn refresh_file(db: &impl Db, path: &Path) -> Result<()> {
    let Some(entry) = entry_for(path) else {
        return remove(&*db.hold()?, path);
    };
    let change = change_for(&*db.hold()?, &entry)?;
    if let Change::Unchanged(_) = change {
        return Ok(());
    }
    let content = content_for(&entry);
    let conn = db.hold()?;
    let change = change_for(&conn, &entry)?;
    write(&conn, &entry, change, content).map(|_| ())
}

pub fn remove(conn: &Connection, path: &Path) -> Result<()> {
    let text = path.to_string_lossy().into_owned();
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM files WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'")?;
        let like = format!("{}\\%", escape_like(&text));
        let rows = stmt.query_map(params![text, like], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<i64>, _>>()?
    };
    if ids.is_empty() {
        return Ok(());
    }
    // A row and its search text go together (audit T-48).
    atomically(conn, "index_remove", |conn| {
        for id in &ids {
            run(conn, "DELETE FROM files_fts WHERE rowid = ?1", [id])?;
            run(conn, "DELETE FROM files WHERE id = ?1", [id])?;
        }
        Ok(())
    })
}

fn escape_like(text: &str) -> String {
    text.replace('!', "!!").replace('%', "!%").replace('_', "!_")
}

/// Records a move or rename the app performed, keeping origin and pins. A
/// file the index did not know yet is read and written here.
pub fn relocate(conn: &Connection, from: &Path, to: &Path) -> Result<()> {
    if !relocate_row(conn, from, to)? {
        refresh_path(conn, to)?;
    }
    Ok(())
}

/// `relocate` for a caller that gives the lock back between steps (a move,
/// an Undo): the row follows the file under one short hold, and a file the
/// index did not know yet is read with no lock held (`refresh_file`).
pub fn relocate_unlocked(db: &impl Db, from: &Path, to: &Path) -> Result<()> {
    let known = relocate_row(&*db.hold()?, from, to)?;
    if !known {
        refresh_file(db, to)?;
    }
    Ok(())
}

/// Moves the index row, its search name, a pin and a template from `from`
/// to `to` in one savepoint, so a crash leaves all of it at one path or the
/// other (audit T-48). Returns false when the index had no row for `from`.
pub fn relocate_row(conn: &Connection, from: &Path, to: &Path) -> Result<bool> {
    let from_text = from.to_string_lossy().into_owned();
    let to_text = to.to_string_lossy().into_owned();
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = to.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = sorter::extension(to);
    let parent = to.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = sorter::classify(&ext).as_str();
    let known = atomically(conn, "index_relocate", |conn| {
        let changed = run(
            conn,
            "UPDATE files SET path = ?2, parent = ?3, name = ?4, ext = ?5, kind = ?6, group_key = ?7 WHERE path = ?1",
            params![from_text, to_text, parent, name, ext, kind, dupes::group_key(&stem, &ext)],
        )?;
        if changed > 0 {
            let id: i64 = conn.query_row("SELECT id FROM files WHERE path = ?1", [&to_text], |r| r.get(0))?;
            run(conn, "UPDATE files_fts SET name = ?2 WHERE rowid = ?1", params![id, indexed_name(&name)])?;
        }
        run(conn, "UPDATE pins SET path = ?2 WHERE path = ?1", [&from_text, &to_text])?;
        run(conn, "UPDATE templates SET path = ?2 WHERE path = ?1", [&from_text, &to_text])?;
        Ok(changed > 0)
    })?;
    // Files attached to a task follow too. A move must never fail because of
    // the list (for example when tasks.sqlite could not be opened), so a
    // problem here is logged and the move stands.
    if let Err(error) = conn.execute("UPDATE OR IGNORE todo.task_files SET path = ?2 WHERE path = ?1", [&from_text, &to_text]) {
        log::warn!("task attachments not updated for {}: {error}", to.display());
    }
    Ok(known)
}

/// Rows a crash left without their search text (audit T-48: the row and its
/// text used to be written one after the other): the name goes back in, so
/// Find lists the file again, and the row is marked changed, so the next
/// scan reads its words again. Runs at start, before anything else writes.
/// Returns how many rows were repaired.
pub fn repair_unsearchable(conn: &Connection) -> Result<usize> {
    let rows: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, name FROM files WHERE id IN (SELECT id FROM files EXCEPT SELECT rowid FROM files_fts)")?;
        let found = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        found.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if rows.is_empty() {
        return Ok(0);
    }
    atomically(conn, "index_repair", |conn| {
        for (id, name) in &rows {
            run(conn, "INSERT INTO files_fts(rowid, name, content) VALUES(?1, ?2, '')", params![id, indexed_name(name)])?;
            run(conn, "UPDATE files SET modified = -1, content_indexed = 0 WHERE id = ?1", [id])?;
        }
        Ok(())
    })?;
    Ok(rows.len())
}

pub fn set_origin(conn: &Connection, path: &Path, origin: &Origin, downloaded_at: i64) -> Result<()> {
    run(
        conn,
        "UPDATE files SET origin_label = ?2, origin_host = ?3, downloaded_at = ?4 WHERE path = ?1",
        params![path.to_string_lossy().into_owned(), origin.label, origin.host, downloaded_at],
    )?;
    Ok(())
}

pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
    Ok(n as usize)
}

/// Removes rows under `root` that the latest walk did not see: files that
/// are gone, and files inside folders the scanner now skips.
pub fn prune_except(conn: &Connection, root: &Path, seen: &std::collections::HashSet<String>) -> Result<usize> {
    let like = format!("{}\\%", escape_like(&root.to_string_lossy()));
    let paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM files WHERE path LIKE ?1 ESCAPE '!'")?;
        let rows = stmt.query_map([like], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<String>, _>>()?
    };
    let mut removed = 0;
    for p in paths {
        if !seen.contains(&p) {
            remove(conn, Path::new(&p))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Marks a file as a download when the browser stamped it and it arrived
/// within `max_age_secs`, wherever she saved it (spec 5.1.3). The file is not
/// moved; it only joins Recent downloads with its source. Returns true when marked.
pub fn note_if_downloaded(conn: &Connection, path: &Path, max_age_secs: i64) -> Result<bool> {
    let path_text = path.to_string_lossy().into_owned();
    let unmarked: Option<i64> = conn
        .query_row("SELECT id FROM files WHERE path = ?1 AND downloaded_at IS NULL", [&path_text], |r| r.get(0))
        .optional()?;
    if unmarked.is_none() {
        return Ok(false);
    }
    let Some(entry) = entry_for(path) else { return Ok(false) };
    if db::now() - entry.created > max_age_secs {
        return Ok(false);
    }
    let Some(origin) = crate::origin::read_zone(path) else { return Ok(false) };
    set_origin(conn, path, &origin, entry.created)?;
    Ok(true)
}

/// The columns every `FileRow` reads, in `row_from` order, and where they come from.
const COLUMNS: &str = "f.id, f.path, f.parent, f.name, f.ext, f.kind, f.size, f.modified, f.origin_label, f.downloaded_at, f.group_key, (p.path IS NOT NULL)";
const FROM: &str = "FROM files f LEFT JOIN pins p ON p.path = f.path";

fn row_from(row: &rusqlite::Row<'_>, snippet: Option<String>) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: row.get(0)?,
        path: row.get(1)?,
        parent: row.get(2)?,
        name: row.get(3)?,
        ext: row.get(4)?,
        kind: row.get(5)?,
        size: row.get::<_, i64>(6)? as u64,
        modified: row.get(7)?,
        origin_label: row.get(8)?,
        downloaded_at: row.get(9)?,
        group_key: row.get(10)?,
        snippet,
        pinned: row.get::<_, i64>(11)? != 0,
    })
}

/// Snippet marks around a matched word. Private-use characters, so brackets
/// or any other text inside a document can never be taken for a match; the
/// renderer turns them into <mark> and `write` keeps them out of the index.
pub const MARK_START: char = '\u{E000}';
pub const MARK_END: char = '\u{E001}';

/// What she typed, cut into words the way the index cuts names and contents:
/// on every character that is not a letter or digit (the unicode61 tokenizer
/// in db.rs). "Book1.xlsx" is "book1" and "xlsx", "Teacher's" is "teacher"
/// and "s", "2026-10-14" is three words.
pub fn words(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// The text the index holds for a file name: the name itself and, when it
/// has an apostrophe, the same name with the apostrophes taken out. The
/// tokenizer cuts "Teacher's" into "teacher" and "s", which a typed
/// "teachers" never matches, so the second form is what finds it. The
/// renderer's `indexedName` (src/lib/search.ts) does the same for the mock.
pub fn indexed_name(name: &str) -> String {
    const APOSTROPHES: [char; 3] = ['\'', '\u{2019}', '\u{2018}'];
    if name.contains(APOSTROPHES) {
        format!("{name} {}", name.replace(APOSTROPHES, ""))
    } else {
        name.to_string()
    }
}

/// Turns what she typed into an FTS5 prefix query: every word must match.
/// None when there is no word at all (empty, or only punctuation).
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = words(input).iter().map(|w| format!("\"{w}\"*")).collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// True when every typed word starts a word of the name (in either indexed
/// form): the name column matched on its own, so the row needs no snippet.
fn name_matches(name: &str, query_words: &[String]) -> bool {
    let name_words = words(&indexed_name(name));
    query_words.iter().all(|q| name_words.iter().any(|n| n.starts_with(q.as_str())))
}

fn filter_sql(filters: &Filters, args: &mut Vec<Box<dyn rusqlite::ToSql>>) -> String {
    let mut sql = String::new();
    if let Some(from) = filters.from {
        args.push(Box::new(from));
        sql.push_str(&format!(" AND COALESCE(f.downloaded_at, f.modified) >= ?{}", args.len()));
    }
    if let Some(to) = filters.to {
        args.push(Box::new(to));
        sql.push_str(&format!(" AND COALESCE(f.downloaded_at, f.modified) < ?{}", args.len()));
    }
    if !filters.kinds.is_empty() {
        let mut marks = Vec::new();
        for kind in &filters.kinds {
            args.push(Box::new(kind.clone()));
            marks.push(format!("?{}", args.len()));
        }
        sql.push_str(&format!(" AND f.kind IN ({})", marks.join(", ")));
    }
    if !filters.sources.is_empty() {
        let mut marks = Vec::new();
        let mut other = false;
        for source in &filters.sources {
            if source == "Other" {
                other = true;
                continue;
            }
            args.push(Box::new(source.clone()));
            marks.push(format!("?{}", args.len()));
        }
        let known = "'Messenger', 'Gmail', 'Google Drive', 'DepEd'";
        match (marks.is_empty(), other) {
            (false, false) => sql.push_str(&format!(" AND f.origin_label IN ({})", marks.join(", "))),
            (false, true) => sql.push_str(&format!(
                " AND (f.origin_label IN ({}) OR f.origin_label NOT IN ({known}))",
                marks.join(", ")
            )),
            (true, true) => sql.push_str(&format!(" AND f.origin_label IS NOT NULL AND f.origin_label NOT IN ({known})")),
            (true, false) => {}
        }
    }
    sql
}

/// Ranked search. A file whose name holds every word comes before one that
/// only mentions them inside, then bm25, then recency. Typed text with no
/// word in it (empty, or only punctuation) browses the newest files that fit
/// the chips; `commands::find` keeps a wordless box with no chips out of
/// search, so the chips alone decide what such a box shows.
pub fn search(conn: &Connection, input: &str, filters: &Filters, limit: usize) -> Result<Vec<FileRow>> {
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let rows = match fts_query(input) {
        Some(query) => {
            // ?2 is the same words restricted to the name column.
            args.push(Box::new(format!("name : ({query})")));
            args.push(Box::new(query));
            let extra = filter_sql(filters, &mut args);
            args.push(Box::new(limit as i64));
            // One query: the ranked ids first, then the rows and their
            // snippets for those ids only. snippet() in a plain select list
            // would run for every match before the limit cut them, reading
            // a whole document each time (audit T-11).
            let sql = format!(
                "WITH hits AS (SELECT f.id AS id, (f.id IN (SELECT rowid FROM files_fts WHERE files_fts MATCH ?1)) AS named, bm25(files_fts, 20.0, 1.0) AS score, f.modified AS modified \
                 FROM files f JOIN files_fts ON files_fts.rowid = f.id WHERE files_fts MATCH ?2{extra} ORDER BY named DESC, score, modified DESC LIMIT ?{}) \
                 SELECT {COLUMNS}, snippet(files_fts, 1, '{MARK_START}', '{MARK_END}', ' ... ', 12) {FROM} \
                 JOIN hits ON hits.id = f.id JOIN files_fts ON files_fts.rowid = f.id AND files_fts MATCH ?2 ORDER BY hits.named DESC, hits.score, hits.modified DESC",
                args.len()
            );
            let mut stmt = conn.prepare(&sql)?;
            let params = rusqlite::params_from_iter(args.iter().map(|a| a.as_ref()));
            let mapped = stmt.query_map(params, |r| {
                let snippet: Option<String> = r.get(12)?;
                row_from(r, snippet)
            })?;
            let mut out = mapped.collect::<std::result::Result<Vec<FileRow>, _>>()?;
            // A name that explains the hit needs no snippet.
            let query_words = words(input);
            for row in &mut out {
                row.snippet = row.snippet.take().filter(|s| !s.trim().is_empty() && !name_matches(&row.name, &query_words));
            }
            out
        }
        None => {
            let extra = filter_sql(filters, &mut args);
            args.push(Box::new(limit as i64));
            let sql = format!("SELECT {COLUMNS} {FROM} WHERE 1 = 1{extra} ORDER BY COALESCE(f.downloaded_at, f.modified) DESC LIMIT ?{}", args.len());
            let mut stmt = conn.prepare(&sql)?;
            let params = rusqlite::params_from_iter(args.iter().map(|a| a.as_ref()));
            let mapped = stmt.query_map(params, |r| row_from(r, None))?;
            mapped.collect::<std::result::Result<Vec<FileRow>, _>>()?
        }
    };
    Ok(rows)
}

/// Files that arrived through the sorter, newest first, within `days`.
pub fn recent_downloads(conn: &Connection, days: i64, limit: usize) -> Result<Vec<FileRow>> {
    let since = db::now() - days * 86_400;
    let sql = format!("SELECT {COLUMNS} {FROM} WHERE f.downloaded_at IS NOT NULL AND f.downloaded_at >= ?1 ORDER BY f.downloaded_at DESC LIMIT ?2");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![since, limit as i64], |r| row_from(r, None))?;
    Ok(rows.collect::<std::result::Result<Vec<FileRow>, _>>()?)
}

pub fn pinned(conn: &Connection) -> Result<Vec<FileRow>> {
    let sql = format!("SELECT {COLUMNS} {FROM} WHERE p.path IS NOT NULL ORDER BY p.created_at DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| row_from(r, None))?;
    Ok(rows.collect::<std::result::Result<Vec<FileRow>, _>>()?)
}

/// Pins whose files are gone from the index (moved or deleted outside the app).
pub fn orphan_pins(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT p.path FROM pins p LEFT JOIN files f ON f.path = p.path WHERE f.id IS NULL ORDER BY p.created_at DESC")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<Vec<String>, _>>()?)
}

pub fn set_pinned(conn: &Connection, path: &str, pinned: bool) -> Result<()> {
    if pinned {
        run(conn, "INSERT OR IGNORE INTO pins(path, created_at) VALUES(?1, ?2)", params![path, db::now()])?;
    } else {
        run(conn, "DELETE FROM pins WHERE path = ?1", [path])?;
    }
    Ok(())
}

pub fn by_path(conn: &Connection, path: &str) -> Result<Option<FileRow>> {
    let sql = format!("SELECT {COLUMNS} {FROM} WHERE f.path = ?1");
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt.query_row([path], |r| row_from(r, None)).optional()?)
}

/// Every file with the same group key, for the "other copies" list.
pub fn group_members(conn: &Connection, group_key: &str) -> Result<Vec<FileRow>> {
    let sql = format!("SELECT {COLUMNS} {FROM} WHERE f.group_key = ?1 ORDER BY f.modified DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([group_key], |r| row_from(r, None))?;
    Ok(rows.collect::<std::result::Result<Vec<FileRow>, _>>()?)
}

/// Groups rows by duplicate key, keeping first-seen order and marking the
/// most recently modified member as Latest.
pub fn group(rows: Vec<FileRow>) -> Vec<Group> {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, Vec<FileRow>> = HashMap::new();
    for row in rows {
        if !buckets.contains_key(&row.group_key) {
            order.push(row.group_key.clone());
        }
        buckets.entry(row.group_key.clone()).or_default().push(row);
    }
    order
        .into_iter()
        .filter_map(|key| {
            let mut members = buckets.remove(&key)?;
            members.sort_by_key(|m| std::cmp::Reverse(m.modified));
            let mut iter = members.into_iter();
            let latest = iter.next()?;
            Some(Group { latest, others: iter.collect() })
        })
        .collect()
}

/// How many files were sorted since `since`, for the one quiet status line.
pub fn sorted_since(conn: &Connection, since: i64) -> Result<usize> {
    // Files Tote itself filed, not downloads it merely noticed elsewhere.
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM ops WHERE kind = 'autoSort' AND status = 'done' AND created_at >= ?1",
        [since],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn seed(dir: &Path) -> Connection {
        let conn = db::open_memory().unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("Grade 7 Quiz.docx"), b"x").unwrap();
        std::fs::write(dir.join("Grade 7 Quiz (2).docx"), b"xy").unwrap();
        std::fs::write(dir.join("sub").join("notes.txt"), "Santos Reyes attendance").unwrap();
        let mut entries = Vec::new();
        walk(dir, &mut entries);
        for e in &entries {
            upsert(&conn, e).unwrap();
        }
        conn
    }

    #[test]
    fn walk_upsert_search_and_group() {
        let dir = tempfile::tempdir().unwrap();
        let conn = seed(dir.path());
        assert_eq!(count(&conn).unwrap(), 3);
        let hits = search(&conn, "quiz", &Filters::default(), 50).unwrap();
        assert_eq!(hits.len(), 2);
        let groups = group(hits);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].others.len(), 1);
        let content = search(&conn, "santos", &Filters::default(), 50).unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].name, "notes.txt");
        assert!(content[0].snippet.as_deref().unwrap().contains(&format!("{MARK_START}Santos{MARK_END}")));
        let kinds = Filters { kinds: vec!["word".into()], ..Default::default() };
        assert_eq!(search(&conn, "", &kinds, 50).unwrap().len(), 2);
    }

    /// A pin, a move's index update or a removal that fails says not
    /// saved, never not read (audit T-18); reads still answer.
    #[test]
    fn a_failed_index_write_says_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let conn = seed(dir.path());
        let from = dir.path().join("Grade 7 Quiz.docx");
        conn.pragma_update(None, "query_only", "ON").unwrap();
        let not_saved = (crate::error::ErrorCode::Db, crate::error::DB_WRITE);
        let error = set_pinned(&conn, &from.to_string_lossy(), true).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), not_saved, "set_pinned");
        let error = relocate(&conn, &from, &dir.path().join("Moved.docx")).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), not_saved, "relocate");
        let error = remove(&conn, &from).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), not_saved, "remove");
        assert!(by_path(&conn, &from.to_string_lossy()).unwrap().is_some(), "reads still work");
        conn.pragma_update(None, "query_only", "OFF").unwrap();
        assert!(set_pinned(&conn, &from.to_string_lossy(), true).is_ok());
    }

    #[test]
    fn relocate_remove_and_pins() {
        let dir = tempfile::tempdir().unwrap();
        let conn = seed(dir.path());
        let from = dir.path().join("Grade 7 Quiz.docx");
        let to = dir.path().join("sub").join("Renamed.docx");
        set_pinned(&conn, &from.to_string_lossy(), true).unwrap();
        std::fs::rename(&from, &to).unwrap();
        relocate(&conn, &from, &to).unwrap();
        let row = by_path(&conn, &to.to_string_lossy()).unwrap().unwrap();
        assert!(row.pinned);
        assert_eq!(row.name, "Renamed.docx");
        assert_eq!(search(&conn, "renamed", &Filters::default(), 50).unwrap().len(), 1);
        remove(&conn, &dir.path().join("sub")).unwrap();
        assert_eq!(count(&conn).unwrap(), 1);
        assert_eq!(orphan_pins(&conn).unwrap().len(), 1);
    }

    /// A move's index update is all or nothing (audit T-48): when one of its
    /// steps fails, the row, its search name and the pin all stay at the old
    /// path; once nothing stands in the way, all of them move.
    #[test]
    fn a_relocate_that_fails_halfway_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let conn = seed(dir.path());
        let from = dir.path().join("Grade 7 Quiz.docx");
        let to = dir.path().join("sub").join("Science Quiz.docx");
        set_pinned(&conn, &from.to_string_lossy(), true).unwrap();
        // The file is also on the template shelf, and the shelf refuses to change.
        conn.execute("INSERT INTO templates(path, name, created_at) VALUES(?1, 'Grade 7 Quiz', 1)", [from.to_string_lossy().into_owned()]).unwrap();
        conn.execute_batch("CREATE TRIGGER no_template_move BEFORE UPDATE ON templates BEGIN SELECT RAISE(ABORT, 'refused'); END;").unwrap();
        assert!(relocate_row(&conn, &from, &to).is_err());
        let row = by_path(&conn, &from.to_string_lossy()).unwrap().expect("the row stayed at the old path");
        assert!(row.pinned, "and so did the pin");
        assert!(by_path(&conn, &to.to_string_lossy()).unwrap().is_none());
        assert_eq!(search(&conn, "science", &Filters::default(), 10).unwrap().len(), 0, "the search name did not change alone");
        assert!(conn.is_autocommit(), "no transaction is left open");
        conn.execute_batch("DROP TRIGGER no_template_move").unwrap();
        assert!(relocate_row(&conn, &from, &to).unwrap(), "the row was known");
        assert!(by_path(&conn, &to.to_string_lossy()).unwrap().unwrap().pinned);
        assert_eq!(search(&conn, "science", &Filters::default(), 10).unwrap().len(), 1);
    }

    /// A row a crash left without its search text cannot be found, and the
    /// next scan would call it unchanged; the start-up repair puts its name
    /// back and marks it changed, so the scan reads its words again.
    #[test]
    fn a_row_left_without_its_search_text_is_repaired_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let conn = seed(dir.path());
        let notes = dir.path().join("sub").join("notes.txt");
        let id = by_path(&conn, &notes.to_string_lossy()).unwrap().unwrap().id;
        conn.execute("DELETE FROM files_fts WHERE rowid = ?1", [id]).unwrap();
        assert!(search(&conn, "notes", &Filters::default(), 10).unwrap().is_empty(), "lost to search");
        let entry = entry_for(&notes).unwrap();
        assert_eq!(change_for(&conn, &entry).unwrap(), Change::Unchanged(id), "and the scan would leave it so");
        assert_eq!(repair_unsearchable(&conn).unwrap(), 1);
        assert_eq!(search(&conn, "notes", &Filters::default(), 10).unwrap().len(), 1, "the name finds it again");
        assert_eq!(change_for(&conn, &entry).unwrap(), Change::Modified(id), "the next scan reads its words again");
        upsert(&conn, &entry).unwrap();
        assert_eq!(search(&conn, "santos", &Filters::default(), 10).unwrap().len(), 1);
        assert_eq!(repair_unsearchable(&conn).unwrap(), 0, "nothing else needed it");
    }

    /// The three steps through a `Db` that is given back between them: a
    /// new file is written with its words, a changed file is refreshed, a
    /// gone file is removed.
    #[test]
    fn refresh_file_indexes_with_the_lock_given_back() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_memory().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "Santos attendance").unwrap();
        refresh_file(&conn, &file).unwrap();
        assert_eq!(search(&conn, "santos", &Filters::default(), 10).unwrap().len(), 1);
        std::fs::write(&file, "Reyes attendance, longer").unwrap();
        refresh_file(&conn, &file).unwrap();
        assert_eq!(search(&conn, "reyes", &Filters::default(), 10).unwrap().len(), 1);
        assert_eq!(count(&conn).unwrap(), 1);
        std::fs::remove_file(&file).unwrap();
        refresh_file(&conn, &file).unwrap();
        assert_eq!(count(&conn).unwrap(), 0);
    }

    #[test]
    fn three_step_upsert_matches_the_one_step_one() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_memory().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "Santos attendance").unwrap();
        let entry = entry_for(&file).unwrap();
        assert_eq!(change_for(&conn, &entry).unwrap(), Change::New);
        let content = content_for(&entry);
        assert_eq!(content.as_deref(), Some("Santos attendance"));
        let id = write(&conn, &entry, Change::New, content).unwrap();
        assert_eq!(change_for(&conn, &entry).unwrap(), Change::Unchanged(id));
        assert_eq!(search(&conn, "santos", &Filters::default(), 10).unwrap().len(), 1);
        std::fs::write(&file, "Reyes attendance, longer now").unwrap();
        let entry = entry_for(&file).unwrap();
        assert!(matches!(change_for(&conn, &entry).unwrap(), Change::Modified(i) if i == id));
        assert_eq!(upsert(&conn, &entry).unwrap(), id);
        assert_eq!(search(&conn, "reyes", &Filters::default(), 10).unwrap().len(), 1);
        assert_eq!(search(&conn, "santos", &Filters::default(), 10).unwrap().len(), 0);
    }

    #[test]
    fn build_output_is_skipped_only_beside_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("app");
        std::fs::create_dir_all(project.join("target").join("release")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(project.join("target").join("release").join("junk.lib"), "x").unwrap();
        std::fs::write(project.join("main.rs"), "fn main() {}").unwrap();
        let teacher = dir.path().join("Lessons").join("build");
        std::fs::create_dir_all(&teacher).unwrap();
        std::fs::write(teacher.join("Robotics Plan.docx"), "x").unwrap();
        let mut entries = Vec::new();
        walk(dir.path(), &mut entries);
        let names: Vec<String> = entries.iter().map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"main.rs".to_string()));
        assert!(names.contains(&"Robotics Plan.docx".to_string()), "a teacher's own 'build' folder stays");
        assert!(!names.contains(&"junk.lib".to_string()), "target beside Cargo.toml is skipped");
        let roots = vec![dir.path().to_path_buf()];
        assert!(under_skipped_dir(&project.join("target").join("release").join("junk.lib"), &roots));
        assert!(!under_skipped_dir(&teacher.join("Robotics Plan.docx"), &roots), "the temp root sits under AppData; that must not count");
    }

    #[test]
    fn prune_except_drops_rows_the_walk_did_not_see() {
        let dir = tempfile::tempdir().unwrap();
        let conn = seed(dir.path());
        let keep = dir.path().join("Grade 7 Quiz.docx").to_string_lossy().into_owned();
        let seen: std::collections::HashSet<String> = [keep.clone()].into_iter().collect();
        assert_eq!(prune_except(&conn, dir.path(), &seen).unwrap(), 2);
        assert_eq!(count(&conn).unwrap(), 1);
        assert!(by_path(&conn, &keep).unwrap().is_some());
    }

    #[cfg(windows)]
    #[test]
    fn stamped_files_anywhere_join_recent_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_memory().unwrap();
        let saved = dir.path().join("Videos").join("Me at the zoo.mp3");
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, "x").unwrap();
        let plain = dir.path().join("Videos").join("my own.mp3");
        std::fs::write(&plain, "x").unwrap();
        std::fs::write(format!("{}:Zone.Identifier", saved.to_string_lossy()), "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/zoo.mp3\r\n").unwrap();
        refresh_path(&conn, &saved).unwrap();
        refresh_path(&conn, &plain).unwrap();
        assert!(note_if_downloaded(&conn, &saved, 600).unwrap());
        assert!(!note_if_downloaded(&conn, &saved, 600).unwrap(), "only once");
        assert!(!note_if_downloaded(&conn, &plain, 600).unwrap(), "no stamp, not a download");
        let recent = recent_downloads(&conn, 30, 10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].origin_label.as_deref(), Some("example.com"));
        assert!(saved.is_file(), "it stays where she saved it");
    }

    #[test]
    fn fts_query_is_prefix_and_quoted() {
        assert_eq!(fts_query("grade 7 \"quiz\"").unwrap(), "\"grade\"* \"7\"* \"quiz\"*");
        assert_eq!(fts_query("   "), None);
    }

    #[test]
    fn fts_query_cuts_words_the_way_names_are_indexed() {
        assert_eq!(fts_query("Book1.xlsx").unwrap(), "\"book1\"* \"xlsx\"*");
        assert_eq!(fts_query("SF2_October").unwrap(), "\"sf2\"* \"october\"*");
        assert_eq!(fts_query("Grade-7").unwrap(), "\"grade\"* \"7\"*");
        assert_eq!(fts_query("Teacher's Day").unwrap(), "\"teacher\"* \"s\"* \"day\"*");
        assert_eq!(fts_query("2026-10-14").unwrap(), "\"2026\"* \"10\"* \"14\"*");
        assert_eq!(fts_query("Dela Cruz, Juan").unwrap(), "\"dela\"* \"cruz\"* \"juan\"*");
        assert_eq!(fts_query("--- ' ..."), None, "punctuation alone is no word");
    }

    #[test]
    fn typed_names_find_their_files_however_they_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_memory().unwrap();
        let names = ["Book1.xlsx", "SF2_October.xlsx", "Grade-7 Science Quiz.docx", "Teacher's Day Program.docx", "2026-10-14 Attendance.xlsx", "Dela Cruz, Juan.pdf"];
        for name in names {
            let path = dir.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            upsert(&conn, &entry_for(&path).unwrap()).unwrap();
        }
        for (typed, expected) in [
            ("Book1.xlsx", "Book1.xlsx"),
            ("SF2_October", "SF2_October.xlsx"),
            ("Grade-7", "Grade-7 Science Quiz.docx"),
            ("Teacher's Day", "Teacher's Day Program.docx"),
            ("teachers day", "Teacher's Day Program.docx"),
            ("2026-10-14", "2026-10-14 Attendance.xlsx"),
            ("Dela Cruz, Juan", "Dela Cruz, Juan.pdf"),
        ] {
            let hits = search(&conn, typed, &Filters::default(), 50).unwrap();
            assert_eq!(hits.first().map(|h| h.name.as_str()), Some(expected), "typed {typed:?}");
            assert!(hits[0].snippet.is_none(), "the name explains the hit for {typed:?}");
        }
        assert_eq!(search(&conn, "", &Filters::default(), 50).unwrap().len(), names.len(), "an empty box with chips still browses");
        // Typed text with no word in it is the same as an empty box: the chip alone decides.
        let excel = Filters { kinds: vec!["excel".into()], ..Default::default() };
        let names_of = |typed: &str| {
            let mut found: Vec<String> = search(&conn, typed, &excel, 50).unwrap().into_iter().map(|h| h.name).collect();
            found.sort();
            found
        };
        let browsed = names_of("");
        assert_eq!(browsed, vec!["2026-10-14 Attendance.xlsx", "Book1.xlsx", "SF2_October.xlsx"], "an empty box with a chip browses the chip's files");
        for wordless in ["---", "'"] {
            assert_eq!(names_of(wordless), browsed, "typed {wordless:?} holds no word, so the chip browses as an empty box does");
        }
    }

    #[test]
    fn a_name_with_an_apostrophe_is_indexed_with_and_without_it() {
        assert_eq!(indexed_name("Teacher's Day.docx"), "Teacher's Day.docx Teachers Day.docx");
        assert_eq!(indexed_name("Juan\u{2019}s Report.pdf"), "Juan\u{2019}s Report.pdf Juans Report.pdf");
        assert_eq!(indexed_name("Book1.xlsx"), "Book1.xlsx", "no apostrophe, nothing added");
        assert!(words(&indexed_name("Teacher's Day.docx")).contains(&"teachers".to_string()));
    }

    #[test]
    fn snippet_marks_cannot_come_from_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_memory().unwrap();
        let form = dir.path().join("form.txt");
        std::fs::write(&form, "[Name] [Section] Santos Reyes attendance").unwrap();
        upsert(&conn, &entry_for(&form).unwrap()).unwrap();
        let hits = search(&conn, "name", &Filters::default(), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet.as_deref(), Some(format!("[{MARK_START}Name{MARK_END}] [Section] Santos Reyes attendance").as_str()), "brackets stay plain text");
        let faked = dir.path().join("faked.txt");
        std::fs::write(&faked, format!("Reyes {MARK_START}Santos{MARK_END} list")).unwrap();
        upsert(&conn, &entry_for(&faked).unwrap()).unwrap();
        let hits = search(&conn, "reyes", &Filters::default(), 10).unwrap();
        let snippet = hits.iter().find(|h| h.name == "faked.txt").unwrap().snippet.clone().unwrap();
        assert_eq!(snippet.matches(MARK_START).count(), 1, "only the match is marked: {snippet:?}");
        assert!(snippet.starts_with(&format!("{MARK_START}Reyes{MARK_END} ")), "{snippet:?}");
    }

    #[test]
    fn names_come_before_contents_whatever_bm25_says() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_memory().unwrap();
        // A long name scores low in bm25; a short note that says the word
        // twenty times scores high. The name still comes first.
        let named = dir.path().join("Santos Attendance Record Grade 7 Mabini First Quarter 2026.xlsx");
        std::fs::write(&named, b"x").unwrap();
        let inside = dir.path().join("notes.txt");
        std::fs::write(&inside, "Santos ".repeat(20)).unwrap();
        let other = dir.path().join("other.txt");
        std::fs::write(&other, "attendance list").unwrap();
        for path in [&named, &inside, &other] {
            upsert(&conn, &entry_for(path).unwrap()).unwrap();
        }
        let hits = search(&conn, "santos", &Filters::default(), 10).unwrap();
        let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["Santos Attendance Record Grade 7 Mabini First Quarter 2026.xlsx", "notes.txt"]);
        assert!(hits[0].snippet.is_none(), "the name explains the hit");
        assert!(hits[1].snippet.is_some(), "the content hit shows where the word is");
    }

    /// The scan writes a chunk in one transaction; its rows and search
    /// answers match the one-step path, for new files and changed ones,
    /// and a chunk that cannot be written leaves the index as it was
    /// (audit T-11).
    #[test]
    fn the_chunked_write_matches_the_one_step_path() {
        type Row = (String, String, i64, i64, i64, String, String);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("Grade 7 Quiz.docx"), b"x").unwrap();
        std::fs::write(dir.path().join("sub").join("notes.txt"), "Santos Reyes attendance").unwrap();
        std::fs::write(dir.path().join("list.csv"), "Dela Cruz, Juan").unwrap();
        let one_step = db::open_memory().unwrap();
        let chunked = db::open_memory().unwrap();
        let both = |entries: &[Entry]| {
            for e in entries {
                upsert(&one_step, e).unwrap();
            }
            let prepared = entries.iter().map(|e| Prepared { entry: e, change: change_for(&chunked, e).unwrap(), content: content_for(e) }).collect();
            write_batch(&chunked, prepared).unwrap()
        };
        let rows = |conn: &Connection| -> Vec<Row> {
            let mut stmt = conn
                .prepare("SELECT f.path, f.name, f.size, f.modified, f.content_indexed, t.name, t.content FROM files f JOIN files_fts t ON t.rowid = f.id ORDER BY f.path")
                .unwrap();
            let found = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))).unwrap();
            found.collect::<std::result::Result<Vec<Row>, _>>().unwrap()
        };
        let mut entries = Vec::new();
        walk(dir.path(), &mut entries);
        assert_eq!(both(&entries), 3, "every file of a new chunk is written");
        assert_eq!(rows(&one_step), rows(&chunked));
        assert_eq!(rows(&chunked).len(), 3);
        std::fs::write(dir.path().join("sub").join("notes.txt"), "Reyes attendance, longer now").unwrap();
        let mut entries = Vec::new();
        walk(dir.path(), &mut entries);
        assert_eq!(both(&entries), 1, "only the changed file is written again");
        assert_eq!(rows(&one_step), rows(&chunked));
        for typed in ["reyes", "santos", "quiz", "juan"] {
            let names = |conn: &Connection| -> Vec<String> { search(conn, typed, &Filters::default(), 10).unwrap().into_iter().map(|h| h.name).collect() };
            assert_eq!(names(&one_step), names(&chunked), "typed {typed:?}");
        }
        // A chunk the database refuses (here read-only) is logged, not
        // half-written: the rows are as they were.
        let before = rows(&chunked);
        chunked.pragma_update(None, "query_only", "ON").unwrap();
        std::fs::write(dir.path().join("list.csv"), "Dela Cruz, Juan, Mabini").unwrap();
        let mut entries = Vec::new();
        walk(dir.path(), &mut entries);
        let prepared = entries.iter().map(|e| Prepared { entry: e, change: change_for(&chunked, e).unwrap(), content: content_for(e) }).collect();
        assert_eq!(write_batch(&chunked, prepared).unwrap(), 0);
        chunked.pragma_update(None, "query_only", "OFF").unwrap();
        assert_eq!(rows(&chunked), before);
        // A file whose write fails costs only itself. Here the race the
        // watcher can cause: a chunk holds two files as New, the watcher
        // indexed both meanwhile, and the first clashes. The chunk is
        // written again without it and with the lookups refreshed, so the
        // second is skipped as up to date and the others go in; the
        // clashing file keeps its row (edited since, it is the next scan's).
        std::fs::write(dir.path().join("Grade 8 Quiz.docx"), b"new").unwrap();
        std::fs::write(dir.path().join("Grade 7 Quiz.docx"), b"edited since").unwrap();
        let mut entries = Vec::new();
        walk(dir.path(), &mut entries);
        let stale = |e: &Entry| if e.path.ends_with("Grade 7 Quiz.docx") || e.path.ends_with("notes.txt") { Change::New } else { change_for(&chunked, e).unwrap() };
        let prepared = entries.iter().map(|e| Prepared { entry: e, change: stale(e), content: content_for(e) }).collect();
        assert_eq!(write_batch(&chunked, prepared).unwrap(), 2, "the changed list and the new quiz; the clashing quiz is left out and the up-to-date notes are skipped");
        assert_eq!(rows(&chunked).len(), 4);
        assert_eq!(rows(&chunked).iter().find(|row| row.1 == "Grade 7 Quiz.docx").unwrap().2, 1, "the clashing file keeps the row the watcher wrote");
        assert!(chunked.is_autocommit());
    }

    /// A release that fails (here a constraint SQLite checks at commit; on
    /// her laptop a full disk) must not leave the savepoint open, or every
    /// later chunk would fail at BEGIN and every watcher write would nest
    /// inside the open transaction and be lost at exit: the row is undone,
    /// the connection is back in autocommit, and the next writes commit.
    #[test]
    fn a_failed_release_leaves_the_connection_usable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Grade 7 Quiz.docx"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "Santos Reyes attendance").unwrap();
        let conn = db::open_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE parent(id INTEGER PRIMARY KEY);
             CREATE TABLE child(parent INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);",
        )
        .unwrap();
        let mut entries = Vec::new();
        walk(dir.path(), &mut entries);
        let quiz = entries.iter().find(|e| e.path.ends_with("Grade 7 Quiz.docx")).unwrap();
        let notes = entries.iter().find(|e| e.path.ends_with("notes.txt")).unwrap();
        let error = write_then(&conn, quiz, Change::New, None, |conn| run(conn, "INSERT INTO child(parent) VALUES(99)", []).map(|_| ())).unwrap_err();
        assert_eq!(error.message, crate::error::DB_WRITE, "the failed release says not saved");
        assert!(conn.is_autocommit(), "the savepoint is not left open");
        assert!(by_path(&conn, &quiz.path.to_string_lossy()).unwrap().is_none(), "the half-written row was undone");
        let later: i64 = conn.query_row("SELECT count(*) FROM child", [], |r| r.get(0)).unwrap();
        assert_eq!(later, 0, "the step that broke the release was undone with it");
        let prepared = vec![Prepared { entry: notes, change: Change::New, content: content_for(notes) }];
        assert_eq!(write_batch(&conn, prepared).unwrap(), 1, "the next chunk still begins and commits");
        assert!(write(&conn, quiz, Change::New, None).is_ok(), "a plain write after it commits too");
        assert_eq!(count(&conn).unwrap(), 2);
        assert_eq!(search(&conn, "santos", &Filters::default(), 10).unwrap().len(), 1);
    }

    /// The scan's merge runs in bounded steps between locks, not in one
    /// 'optimize' that rewrites the whole index while every search waits
    /// (audit T-11): the first step gathers the segments the chunks left,
    /// the later steps carry that merge on, a step says when nothing is
    /// left so the loop ends, and the index answers the same after. One
    /// new segment is left to FTS5 unless a first step asks for it.
    #[test]
    fn the_merge_runs_in_steps_and_says_when_it_is_done() {
        let conn = db::open_memory().unwrap();
        // FTS5's own merging switched off for this table, so the chunks
        // leave exactly one segment each and the steps are predictable.
        conn.execute_batch(
            "INSERT INTO files_fts(files_fts, rank) VALUES('automerge', 0);
             INSERT INTO files_fts(files_fts, rank) VALUES('crisismerge', 1000);",
        )
        .unwrap();
        let entry = |i: i64| Entry { path: PathBuf::from(format!(r"C:\Lab\Grade {} Quiz {i}.docx", i % 12)), size: 10, modified: i, created: i, content_available: true };
        // Forty words of its own per file, so the index is many pages and
        // the merge takes more than one step.
        let words = |i: i64| (0..40).fold(String::from("Santos Reyes attendance"), |mut text, k| {
            text.push_str(&format!(" word{}", i * 40 + k));
            text
        });
        let write_files = |from: i64, to: i64| {
            let entries: Vec<Entry> = (from..to).map(entry).collect();
            for chunk in entries.chunks(25) {
                let prepared = chunk.iter().map(|e| Prepared { entry: e, change: Change::New, content: Some(words(e.modified)) }).collect();
                assert_eq!(write_batch(&conn, prepared).unwrap(), chunk.len());
            }
        };
        write_files(0, 8_000);
        assert_eq!(fts_levels(&conn), vec![(320, 0)], "one segment per chunk of 25, not one per file");
        let segments = |conn: &Connection| fts_levels(conn).iter().map(|(segments, _)| segments).sum::<u64>();
        let names = |conn: &Connection| -> Vec<String> { search(conn, "quiz 7", &Filters::default(), 50).unwrap().into_iter().map(|h| h.name).collect() };
        let before = names(&conn);
        assert_eq!(before.len(), 50);
        assert!(merge_step(&conn, true).unwrap(), "the chunks left segments to merge");
        let mut steps = 1;
        while merge_step(&conn, false).unwrap() {
            steps += 1;
            assert!(steps <= 200, "the merge of 8,000 small files ends");
        }
        assert!(steps >= 2, "an index of many pages takes more than one bounded step ({steps})");
        assert_eq!(segments(&conn), 1, "one segment, as 'optimize' leaves");
        assert!(!merge_step(&conn, false).unwrap(), "nothing left: one more step is a no-op, so a loop ends");
        assert_eq!(names(&conn), before, "the merged index answers the same");
        assert_eq!(search(&conn, "word80001", &Filters::default(), 10).unwrap().len(), 1);
        write_files(8_000, 8_025);
        assert_eq!(segments(&conn), 2);
        assert!(!merge_step(&conn, false).unwrap(), "one new segment beside the merged one is left to FTS5");
        assert!(merge_step(&conn, true).unwrap(), "a first step gathers it in");
        while merge_step(&conn, false).unwrap() {}
        assert_eq!(segments(&conn), 1);
        assert_eq!(search(&conn, "word320001", &Filters::default(), 10).unwrap().len(), 1);
    }

    /// The segments of the search index per level, as (segments, of which
    /// inputs to a merge in progress), read from FTS5's structure record
    /// (fts5_index.c, "structure record": a cookie, then varints).
    fn fts_levels(conn: &Connection) -> Vec<(u64, u64)> {
        let blob: Vec<u8> = conn.query_row("SELECT block FROM files_fts_data WHERE id = 10", [], |r| r.get(0)).unwrap();
        // After the cookie, a V2 record (origin counters in use) carries a
        // mark and five more varints per segment.
        let v2 = blob[4..8] == [0xff, 0x00, 0x00, 0x01];
        let mut pos = if v2 { 8 } else { 4 };
        let mut varint = || {
            let mut value: u64 = 0;
            loop {
                let byte = blob[pos];
                pos += 1;
                value = (value << 7) | u64::from(byte & 0x7f);
                if byte < 0x80 {
                    return value;
                }
            }
        };
        let levels = varint();
        let _segments = varint();
        let _write_counter = varint();
        (0..levels)
            .map(|_| {
                let merging = varint();
                let segments = varint();
                for _ in 0..segments {
                    for _ in 0..(if v2 { 8 } else { 3 }) {
                        varint();
                    }
                }
                (segments, merging)
            })
            .collect()
    }
}
