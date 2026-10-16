//! The search index (spec 5.2 and 5.3): what is on disk, what is inside it,
//! and the ranked, grouped answers to "where is that file".

use crate::db;
use crate::dupes;
use crate::error::Result;
use crate::extract;
use crate::origin::Origin;
use crate::sorter;
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

/// Writes a new or changed row and its searchable text. Returns the row id.
pub fn write(conn: &Connection, entry: &Entry, change: Change, content: Option<String>) -> Result<i64> {
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
            conn.execute(
                "UPDATE files SET size = ?2, modified = ?3, content_indexed = ?4 WHERE id = ?1",
                params![id, entry.size as i64, entry.modified, content.is_some() as i64],
            )?;
            conn.execute("DELETE FROM files_fts WHERE rowid = ?1", [id])?;
            id
        }
        Change::New => {
            conn.execute(
                "INSERT INTO files(path, parent, name, ext, kind, size, modified, group_key, content_indexed) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![path_text, parent, name, ext, kind, entry.size as i64, entry.modified, group_key, content.is_some() as i64],
            )?;
            conn.last_insert_rowid()
        }
    };
    // A document that happens to hold the snippet marks must not light up.
    let content = content.map(|c| if c.contains([MARK_START, MARK_END]) { c.replace([MARK_START, MARK_END], " ") } else { c });
    conn.execute(
        "INSERT INTO files_fts(rowid, name, content) VALUES(?1, ?2, ?3)",
        params![id, indexed_name(&name), content.unwrap_or_default()],
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

pub fn remove(conn: &Connection, path: &Path) -> Result<()> {
    let text = path.to_string_lossy().into_owned();
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM files WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'")?;
        let like = format!("{}\\%", escape_like(&text));
        let rows = stmt.query_map(params![text, like], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<i64>, _>>()?
    };
    for id in ids {
        conn.execute("DELETE FROM files_fts WHERE rowid = ?1", [id])?;
        conn.execute("DELETE FROM files WHERE id = ?1", [id])?;
    }
    Ok(())
}

fn escape_like(text: &str) -> String {
    text.replace('!', "!!").replace('%', "!%").replace('_', "!_")
}

/// Records a move or rename the app performed, keeping origin and pins.
pub fn relocate(conn: &Connection, from: &Path, to: &Path) -> Result<()> {
    let from_text = from.to_string_lossy().into_owned();
    let to_text = to.to_string_lossy().into_owned();
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = to.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = sorter::extension(to);
    let parent = to.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = sorter::classify(&ext).as_str();
    let changed = conn.execute(
        "UPDATE files SET path = ?2, parent = ?3, name = ?4, ext = ?5, kind = ?6, group_key = ?7 WHERE path = ?1",
        params![from_text, to_text, parent, name, ext, kind, dupes::group_key(&stem, &ext)],
    )?;
    if changed == 0 {
        refresh_path(conn, to)?;
    } else {
        let id: i64 = conn.query_row("SELECT id FROM files WHERE path = ?1", [&to_text], |r| r.get(0))?;
        conn.execute("UPDATE files_fts SET name = ?2 WHERE rowid = ?1", params![id, indexed_name(&name)])?;
    }
    conn.execute("UPDATE pins SET path = ?2 WHERE path = ?1", [&from_text, &to_text])?;
    conn.execute("UPDATE templates SET path = ?2 WHERE path = ?1", [&from_text, &to_text])?;
    // Files attached to a task follow too. A move must never fail because of
    // the list (for example when tasks.sqlite could not be opened), so a
    // problem here is logged and the move stands.
    if let Err(error) = conn.execute("UPDATE OR IGNORE todo.task_files SET path = ?2 WHERE path = ?1", [&from_text, &to_text]) {
        log::warn!("task attachments not updated for {}: {error}", to.display());
    }
    Ok(())
}

pub fn set_origin(conn: &Connection, path: &Path, origin: &Origin, downloaded_at: i64) -> Result<()> {
    conn.execute(
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

const SELECT: &str = "SELECT f.id, f.path, f.parent, f.name, f.ext, f.kind, f.size, f.modified, f.origin_label, f.downloaded_at, f.group_key, (p.path IS NOT NULL) FROM files f LEFT JOIN pins p ON p.path = f.path";

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
            let sql = format!(
                "{SELECT} JOIN files_fts t ON t.rowid = f.id WHERE files_fts MATCH ?2{extra} ORDER BY (f.id IN (SELECT rowid FROM files_fts WHERE files_fts MATCH ?1)) DESC, bm25(files_fts, 20.0, 1.0), f.modified DESC LIMIT ?{}",
                args.len()
            );
            let mut stmt = conn.prepare(&sql)?;
            let params = rusqlite::params_from_iter(args.iter().map(|a| a.as_ref()));
            let mapped = stmt.query_map(params, |r| {
                // snippet() needs the fts table in scope; computed in a second query only for content hits.
                row_from(r, None)
            })?;
            let mut out = mapped.collect::<std::result::Result<Vec<FileRow>, _>>()?;
            attach_snippets(conn, input, &mut out)?;
            out
        }
        None => {
            let extra = filter_sql(filters, &mut args);
            args.push(Box::new(limit as i64));
            let sql = format!("{SELECT} WHERE 1 = 1{extra} ORDER BY COALESCE(f.downloaded_at, f.modified) DESC LIMIT ?{}", args.len());
            let mut stmt = conn.prepare(&sql)?;
            let params = rusqlite::params_from_iter(args.iter().map(|a| a.as_ref()));
            let mapped = stmt.query_map(params, |r| row_from(r, None))?;
            mapped.collect::<std::result::Result<Vec<FileRow>, _>>()?
        }
    };
    Ok(rows)
}

/// Adds a content snippet to rows whose name does not already explain the hit.
fn attach_snippets(conn: &Connection, input: &str, rows: &mut [FileRow]) -> Result<()> {
    let Some(query) = fts_query(input) else { return Ok(()) };
    let query_words = words(input);
    let sql = format!("SELECT snippet(files_fts, 1, '{MARK_START}', '{MARK_END}', ' ... ', 12) FROM files_fts WHERE rowid = ?1 AND files_fts MATCH ?2");
    let mut stmt = conn.prepare(&sql)?;
    for row in rows.iter_mut() {
        if name_matches(&row.name, &query_words) {
            continue;
        }
        let snippet: Option<String> = stmt.query_row(params![row.id, query], |r| r.get(0)).optional()?;
        row.snippet = snippet.filter(|s| !s.trim().is_empty());
    }
    Ok(())
}

/// Files that arrived through the sorter, newest first, within `days`.
pub fn recent_downloads(conn: &Connection, days: i64, limit: usize) -> Result<Vec<FileRow>> {
    let since = db::now() - days * 86_400;
    let sql = format!("{SELECT} WHERE f.downloaded_at IS NOT NULL AND f.downloaded_at >= ?1 ORDER BY f.downloaded_at DESC LIMIT ?2");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![since, limit as i64], |r| row_from(r, None))?;
    Ok(rows.collect::<std::result::Result<Vec<FileRow>, _>>()?)
}

pub fn pinned(conn: &Connection) -> Result<Vec<FileRow>> {
    let sql = format!("{SELECT} WHERE p.path IS NOT NULL ORDER BY p.created_at DESC");
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
        conn.execute("INSERT OR IGNORE INTO pins(path, created_at) VALUES(?1, ?2)", params![path, db::now()])?;
    } else {
        conn.execute("DELETE FROM pins WHERE path = ?1", [path])?;
    }
    Ok(())
}

pub fn by_path(conn: &Connection, path: &str) -> Result<Option<FileRow>> {
    let sql = format!("{SELECT} WHERE f.path = ?1");
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt.query_row([path], |r| row_from(r, None)).optional()?)
}

/// Every file with the same group key, for the "other copies" list.
pub fn group_members(conn: &Connection, group_key: &str) -> Result<Vec<FileRow>> {
    let sql = format!("{SELECT} WHERE f.group_key = ?1 ORDER BY f.modified DESC");
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
}
