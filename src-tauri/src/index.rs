//! The search index (spec 5.2 and 5.3): what is on disk, what is inside it,
//! and the ranked, grouped answers to "where is that file".

use crate::db;
use crate::dupes;
use crate::error::{DeskError, Result};
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
    pub content_available: bool,
}

const SKIP_DIRS: &[&str] = &["AppData", "node_modules", ".git", "$RECYCLE.BIN", "System Volume Information"];

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
    Some(Entry { path: path.to_path_buf(), size: meta.len(), modified, content_available: content_present(&meta) })
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
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
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

/// Inserts or refreshes one file. Text is extracted only when the file is new
/// or changed. Returns the row id.
pub fn upsert(conn: &Connection, entry: &Entry) -> Result<i64> {
    let path_text = entry.path.to_string_lossy().into_owned();
    let existing: Option<(i64, i64, i64)> = conn
        .query_row("SELECT id, modified, size FROM files WHERE path = ?1", [&path_text], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    if let Some((id, modified, size)) = existing {
        if modified == entry.modified && size as u64 == entry.size {
            return Ok(id);
        }
    }
    let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = entry.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = sorter::extension(&entry.path);
    let kind = sorter::classify(&ext).as_str();
    let parent = entry.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let group_key = dupes::group_key(&stem, &ext);
    let content = if entry.content_available { extract::extract_text(&entry.path, &ext, entry.size) } else { None };

    let id = match existing {
        Some((id, _, _)) => {
            conn.execute(
                "UPDATE files SET size = ?2, modified = ?3, content_indexed = ?4 WHERE id = ?1",
                params![id, entry.size as i64, entry.modified, content.is_some() as i64],
            )?;
            conn.execute("DELETE FROM files_fts WHERE rowid = ?1", [id])?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO files(path, parent, name, ext, kind, size, modified, group_key, content_indexed) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![path_text, parent, name, ext, kind, entry.size as i64, entry.modified, group_key, content.is_some() as i64],
            )?;
            conn.last_insert_rowid()
        }
    };
    conn.execute(
        "INSERT INTO files_fts(rowid, name, content) VALUES(?1, ?2, ?3)",
        params![id, name, content.unwrap_or_default()],
    )?;
    Ok(id)
}

/// Indexes a path from disk if it is indexable, otherwise removes any stale row.
pub fn refresh_path(conn: &Connection, path: &Path) -> Result<()> {
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
        conn.execute("UPDATE files_fts SET name = ?2 WHERE rowid = ?1", params![id, name])?;
    }
    conn.execute("UPDATE pins SET path = ?2 WHERE path = ?1", [&from_text, &to_text])?;
    conn.execute("UPDATE templates SET path = ?2 WHERE path = ?1", [&from_text, &to_text])?;
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

/// Removes rows whose files no longer exist under `root`.
pub fn prune_missing(conn: &Connection, root: &Path) -> Result<usize> {
    let like = format!("{}\\%", escape_like(&root.to_string_lossy()));
    let paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM files WHERE path LIKE ?1 ESCAPE '!'")?;
        let rows = stmt.query_map([like], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<String>, _>>()?
    };
    let mut removed = 0;
    for p in paths {
        if !Path::new(&p).is_file() {
            remove(conn, Path::new(&p))?;
            removed += 1;
        }
    }
    Ok(removed)
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

/// Turns what she typed into an FTS5 prefix query: every word must match.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
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

/// Ranked search. Name matches outrank content matches; ties break by recency.
pub fn search(conn: &Connection, input: &str, filters: &Filters, limit: usize) -> Result<Vec<FileRow>> {
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let rows = match fts_query(input) {
        Some(query) => {
            args.push(Box::new(query));
            let extra = filter_sql(filters, &mut args);
            args.push(Box::new(limit as i64));
            let sql = format!(
                "{SELECT} JOIN files_fts t ON t.rowid = f.id WHERE files_fts MATCH ?1{extra} ORDER BY bm25(files_fts, 20.0, 1.0), f.modified DESC LIMIT ?{}",
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
    let words: Vec<String> = input.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut stmt = conn.prepare("SELECT snippet(files_fts, 1, '[', ']', ' ... ', 12) FROM files_fts WHERE rowid = ?1 AND files_fts MATCH ?2")?;
    for row in rows.iter_mut() {
        let lower = row.name.to_lowercase();
        if words.iter().all(|w| lower.contains(w)) {
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
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE downloaded_at >= ?1", [since], |r| r.get(0))?;
    Ok(n as usize)
}

pub fn lock_err() -> DeskError {
    DeskError::db("The app state is busy. Try again.")
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
        assert!(content[0].snippet.as_deref().unwrap().contains("[Santos]"));
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
    fn fts_query_is_prefix_and_quoted() {
        assert_eq!(fts_query("grade 7 \"quiz\"").unwrap(), "\"grade\"* \"7\"* \"quiz\"*");
        assert_eq!(fts_query("   "), None);
    }
}
