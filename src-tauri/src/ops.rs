//! Every change to her files (spec 5.4 and 5.7): logged before it happens,
//! confined to the indexed roots, never overwriting, never deleting for good,
//! and undoable afterwards.

use crate::error::{ToteError, ErrorCode, Result};
use crate::index;
use crate::paths::{self, Roots};
use crate::sorter;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    AutoSort,
    Move,
    Rename,
    NewFolder,
    Recycle,
    Copy,
}

impl OpKind {
    fn as_str(self) -> &'static str {
        match self {
            OpKind::AutoSort => "autoSort",
            OpKind::Move => "move",
            OpKind::Rename => "rename",
            OpKind::NewFolder => "newFolder",
            OpKind::Recycle => "recycle",
            OpKind::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRow {
    pub id: i64,
    pub kind: String,
    pub src: String,
    pub dst: Option<String>,
    pub status: String,
    pub message: Option<String>,
    pub automatic: bool,
    pub created_at: i64,
    /// Set when the change was one of several made by one action: an Organize
    /// run, or a Move or Recycle Bin of several ticked files. Activity shows
    /// the batch as one row with Undo all.
    pub batch: Option<String>,
}

/// A fresh batch id such as `recycle-1760300000000-3`. The counter keeps two
/// actions in the same millisecond apart.
pub fn new_batch(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}-{}-{}", chrono::Utc::now().timestamp_millis(), SEQ.fetch_add(1, Ordering::Relaxed))
}

fn record(conn: &Connection, kind: OpKind, src: &Path, dst: Option<&Path>, automatic: bool) -> Result<i64> {
    record_in(conn, kind, src, dst, automatic, None)
}

/// `batch` ties the changes of one action together so one Undo reverses them all.
fn record_in(conn: &Connection, kind: OpKind, src: &Path, dst: Option<&Path>, automatic: bool, batch: Option<&str>) -> Result<i64> {
    conn.execute(
        "INSERT INTO ops(kind, src, dst, status, automatic, created_at, batch) VALUES(?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
        params![
            kind.as_str(),
            src.to_string_lossy().into_owned(),
            dst.map(|d| d.to_string_lossy().into_owned()),
            automatic as i64,
            crate::db::now(),
            batch
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn finish(conn: &Connection, id: i64, result: &Result<PathBuf>) -> Result<()> {
    match result {
        Ok(dst) => {
            conn.execute("UPDATE ops SET status = 'done', dst = ?2 WHERE id = ?1", params![id, dst.to_string_lossy().into_owned()])?;
        }
        Err(error) => {
            conn.execute("UPDATE ops SET status = 'failed', message = ?2 WHERE id = ?1", params![id, error.message])?;
        }
    }
    Ok(())
}

/// Rename when possible; copy, verify the size, then remove the source when
/// the destination is on another volume. A failed verification leaves the
/// original in place and removes the partial copy.
pub fn transfer(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(ToteError::new(ErrorCode::AlreadyExists, "A file with that name already exists there.").with_path(paths::display(dst)));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    let expected = std::fs::metadata(src)?.len();
    let copied = std::fs::copy(src, dst)?;
    if copied != expected || std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0) != expected {
        let _ = std::fs::remove_file(dst);
        return Err(ToteError::new(ErrorCode::Io, "The copy did not complete, so the file was left where it was.").with_path(paths::display(src)));
    }
    std::fs::remove_file(src)?;
    Ok(())
}

/// Moves one file into `dst_dir`, clash rule applied. Returns the new path.
pub fn move_file(conn: &Connection, roots: &Roots, src: &Path, dst_dir: &Path, kind: OpKind) -> Result<PathBuf> {
    move_inner(conn, roots, src, dst_dir, kind, None)
}

/// A move that belongs to an Organize run (spec 5.11).
pub fn move_file_in_batch(conn: &Connection, roots: &Roots, src: &Path, dst_dir: &Path, batch: &str) -> Result<PathBuf> {
    move_inner(conn, roots, src, dst_dir, OpKind::Move, Some(batch))
}

fn move_inner(conn: &Connection, roots: &Roots, src: &Path, dst_dir: &Path, kind: OpKind, batch: Option<&str>) -> Result<PathBuf> {
    let src = roots.confine(src)?;
    let dst_dir = roots.confine(dst_dir)?;
    if !src.is_file() {
        return Err(ToteError::new(ErrorCode::NotFound, "That file is no longer there.").with_path(paths::display(&src)));
    }
    let name = src.file_name().ok_or_else(|| ToteError::new(ErrorCode::InvalidPath, "The file has no name."))?;
    let wanted = dst_dir.join(name);
    if wanted == src {
        return Ok(src);
    }
    let dst = sorter::unique_path(&wanted);
    let id = record_in(conn, kind, &src, Some(&dst), kind == OpKind::AutoSort, batch)?;
    let result = transfer(&src, &dst).map(|_| dst.clone());
    finish(conn, id, &result)?;
    let dst = result?;
    index::relocate(conn, &src, &dst)?;
    Ok(dst)
}

/// Renames a file in place, keeping its extension locked.
pub fn rename_file(conn: &Connection, roots: &Roots, src: &Path, new_stem: &str) -> Result<PathBuf> {
    let src = roots.confine(src)?;
    if !src.is_file() {
        return Err(ToteError::new(ErrorCode::NotFound, "That file is no longer there.").with_path(paths::display(&src)));
    }
    paths::validate_name(new_stem)?;
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let parent = src.parent().ok_or_else(|| ToteError::new(ErrorCode::InvalidPath, "The file has no folder."))?;
    let dst = parent.join(format!("{new_stem}{ext}"));
    if dst == src {
        return Ok(src);
    }
    if dst.exists() {
        return Err(ToteError::new(ErrorCode::AlreadyExists, "A file with that name already exists in this folder.").with_path(paths::display(&dst)));
    }
    let id = record(conn, OpKind::Rename, &src, Some(&dst), false)?;
    let result = std::fs::rename(&src, &dst).map(|_| dst.clone()).map_err(ToteError::from);
    finish(conn, id, &result)?;
    let dst = result?;
    index::relocate(conn, &src, &dst)?;
    Ok(dst)
}

pub fn create_folder(conn: &Connection, roots: &Roots, parent: &Path, name: &str) -> Result<PathBuf> {
    paths::validate_name(name)?;
    let parent = roots.confine(parent)?;
    let dst = parent.join(name);
    if dst.exists() {
        return Err(ToteError::new(ErrorCode::AlreadyExists, "A folder with that name already exists.").with_path(paths::display(&dst)));
    }
    let id = record(conn, OpKind::NewFolder, &dst, None, false)?;
    let result = std::fs::create_dir(&dst).map(|_| dst.clone()).map_err(ToteError::from);
    finish(conn, id, &result)?;
    result
}

/// Sends files to the Recycle Bin, one logged operation per file. Several
/// files sent together share a batch, so Activity shows them as one row.
pub fn recycle(conn: &Connection, roots: &Roots, targets: &[PathBuf]) -> Result<Vec<OpRow>> {
    let batch = (targets.len() > 1).then(|| new_batch("recycle"));
    let mut rows = Vec::new();
    for target in targets {
        let src = roots.confine(target)?;
        if !src.exists() {
            continue;
        }
        let id = record_in(conn, OpKind::Recycle, &src, None, false, batch.as_deref())?;
        let result = trash::delete(&src)
            .map(|_| src.clone())
            .map_err(|e| ToteError::new(ErrorCode::Io, format!("Windows could not move it to the Recycle Bin: {e}")).with_path(paths::display(&src)));
        finish(conn, id, &result)?;
        if result.is_ok() {
            index::remove(conn, &src)?;
        }
        if let Some(row) = get(conn, id)? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// Reverses a completed operation. Moves and renames go back to their original
/// path (clash rule applied); a recycled file is restored from the Recycle Bin.
pub fn undo(conn: &Connection, roots: &Roots, id: i64) -> Result<()> {
    let row = get(conn, id)?.ok_or_else(|| ToteError::new(ErrorCode::NotFound, "That change is no longer in the list."))?;
    if row.status != "done" {
        return Err(ToteError::validation("Only completed changes can be undone."));
    }
    let src = PathBuf::from(&row.src);
    match row.kind.as_str() {
        "autoSort" | "move" | "rename" | "copy" => {
            let current = PathBuf::from(row.dst.as_deref().unwrap_or_default());
            let current = roots.confine(&current)?;
            if row.kind == "copy" {
                // A template copy is undone by recycling the copy; the master stays.
                trash::delete(&current).map_err(|e| ToteError::new(ErrorCode::Io, e.to_string()))?;
                index::remove(conn, &current)?;
            } else {
                let back = sorter::unique_path(&roots.confine(&src)?);
                transfer(&current, &back)?;
                index::relocate(conn, &current, &back)?;
            }
        }
        "newFolder" => {
            let folder = roots.confine(&src)?;
            if std::fs::read_dir(&folder).map(|mut d| d.next().is_some()).unwrap_or(false) {
                return Err(ToteError::validation("That folder has files in it now, so it stays."));
            }
            std::fs::remove_dir(&folder)?;
        }
        "recycle" => {
            restore_from_recycle_bin(&src)?;
            index::refresh_path(conn, &src)?;
        }
        other => return Err(ToteError::new(ErrorCode::Unsupported, format!("Cannot undo {other}."))),
    }
    conn.execute("UPDATE ops SET status = 'undone' WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn restore_from_recycle_bin(original: &Path) -> Result<()> {
    use trash::os_limited;
    let items = os_limited::list().map_err(|e| ToteError::new(ErrorCode::Io, e.to_string()))?;
    let wanted = original.to_string_lossy().to_lowercase();
    // The shell reports the item name the way Explorer shows it, which can be
    // without the extension, so the match accepts the full name or the stem.
    let wanted_parent = original.parent().map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_default();
    let wanted_name = original.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let wanted_stem = original.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut matching: Vec<trash::TrashItem> = items
        .into_iter()
        .filter(|i| {
            let parent = i.original_parent.to_string_lossy().to_lowercase();
            let name = i.name.to_string_lossy().to_lowercase();
            i.original_path().to_string_lossy().to_lowercase() == wanted
                || (parent == wanted_parent && (name == wanted_name || name == wanted_stem))
        })
        .collect();
    if matching.is_empty() {
        log::warn!("restore: nothing in the Recycle Bin has the original path {wanted}");
        return Err(ToteError::new(ErrorCode::NotFound, "That file is not in the Recycle Bin any more. Open the Recycle Bin to check."));
    }
    // Several recycled copies can share one original path; Undo brings back
    // the one recycled most recently (restoring all of them is refused by Windows).
    matching.sort_by_key(|i| i.time_deleted);
    let latest = matching.pop().ok_or_else(|| ToteError::new(ErrorCode::NotFound, "Nothing to restore."))?;
    log::info!("restoring {} from the Recycle Bin to {wanted}", latest.id.to_string_lossy());
    let restored_as = latest.original_path();
    os_limited::restore_all(vec![latest]).map_err(|e| ToteError::new(ErrorCode::Io, format!("Windows could not restore it: {e}")))?;
    // When Explorer hides extensions the item comes back as "Name" instead of
    // "Name.pdf"; put the recorded name back.
    if !original.exists() && restored_as != original && restored_as.exists() {
        std::fs::rename(&restored_as, original)?;
        log::info!("renamed the restored item back to {}", original.display());
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn restore_from_recycle_bin(_original: &Path) -> Result<()> {
    Err(ToteError::new(ErrorCode::Unsupported, "Restoring from the Recycle Bin is not available here."))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUndo {
    pub undone: usize,
    pub failed: Vec<String>,
}

/// Reverses every completed change of one batch, newest first. For an
/// Organize run it then removes the folders the run created that are now
/// empty (never a folder that still holds anything, never School Files
/// itself). A manual Move of several files never removes a folder: it may
/// have existed, empty, before she moved anything into it.
pub fn undo_batch(conn: &Connection, roots: &Roots, batch: &str) -> Result<BatchUndo> {
    let rows: Vec<OpRow> = {
        let mut stmt = conn.prepare("SELECT id, kind, src, dst, status, message, automatic, created_at, batch FROM ops WHERE batch = ?1 AND status = 'done' ORDER BY id DESC")?;
        let mapped = stmt.query_map([batch], map_op)?;
        mapped.collect::<std::result::Result<Vec<OpRow>, _>>()?
    };
    let mut undone = 0;
    let mut failed = Vec::new();
    let mut touched: Vec<PathBuf> = Vec::new();
    for row in rows {
        match undo(conn, roots, row.id) {
            Ok(()) => {
                undone += 1;
                if let Some(parent) = row.dst.as_deref().and_then(|d| Path::new(d).parent()) {
                    touched.push(parent.to_path_buf());
                }
            }
            Err(error) => {
                log::warn!("undo batch {batch}, op {}: {}", row.id, error.message);
                failed.push(Path::new(&row.src).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
        }
    }
    if !batch.starts_with("organize-") {
        return Ok(BatchUndo { undone, failed });
    }
    let school = roots.school_files()?;
    touched.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    touched.dedup();
    for dir in touched {
        let mut current = Some(dir.as_path());
        while let Some(folder) = current {
            if folder == school || !folder.starts_with(&school) {
                break;
            }
            let empty = std::fs::read_dir(folder).map(|mut d| d.next().is_none()).unwrap_or(false);
            if !empty || std::fs::remove_dir(folder).is_err() {
                break;
            }
            current = folder.parent();
        }
    }
    Ok(BatchUndo { undone, failed })
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<OpRow>> {
    let mut stmt = conn.prepare("SELECT id, kind, src, dst, status, message, automatic, created_at, batch FROM ops WHERE id = ?1")?;
    Ok(stmt.query_row([id], map_op).optional()?)
}

pub fn list(conn: &Connection, limit: usize) -> Result<Vec<OpRow>> {
    let mut stmt = conn.prepare("SELECT id, kind, src, dst, status, message, automatic, created_at, batch FROM ops WHERE status <> 'pending' ORDER BY id DESC LIMIT ?1")?;
    let rows = stmt.query_map([limit as i64], map_op)?;
    Ok(rows.collect::<std::result::Result<Vec<OpRow>, _>>()?)
}

/// Writes a failure the sorter hit so Activity can show it.
pub fn note_failure(conn: &Connection, src: &Path, message: &str) -> Result<()> {
    let id = record(conn, OpKind::AutoSort, src, None, true)?;
    conn.execute("UPDATE ops SET status = 'failed', message = ?2 WHERE id = ?1", params![id, message])?;
    Ok(())
}

fn map_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<OpRow> {
    Ok(OpRow {
        id: row.get(0)?,
        kind: row.get(1)?,
        src: row.get(2)?,
        dst: row.get(3)?,
        status: row.get(4)?,
        message: row.get(5)?,
        automatic: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
        batch: row.get(8)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::db;

    fn fixture() -> (tempfile::TempDir, Roots, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let desktop = dir.path().join("Desktop");
        let documents = dir.path().join("Documents");
        let downloads = dir.path().join("Downloads");
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        let roots = Roots::explicit(desktop, documents, downloads);
        let conn = db::open_memory().unwrap();
        (dir, roots, conn)
    }

    #[test]
    fn move_logs_relocates_and_undoes() {
        let (_dir, roots, conn) = fixture();
        let src = roots.downloads.clone().unwrap().join("Quiz.docx");
        std::fs::write(&src, b"q").unwrap();
        index::refresh_path(&conn, &src).unwrap();
        let dest = roots.documents.clone().unwrap().join("School Files").join("Word");
        let moved = move_file(&conn, &roots, &src, &dest, OpKind::Move).unwrap();
        assert!(moved.is_file());
        assert!(!src.exists());
        assert!(index::by_path(&conn, &moved.to_string_lossy()).unwrap().is_some());
        let ops = list(&conn, 10).unwrap();
        assert_eq!(ops[0].status, "done");
        undo(&conn, &roots, ops[0].id).unwrap();
        assert!(src.is_file());
        assert_eq!(get(&conn, ops[0].id).unwrap().unwrap().status, "undone");
    }

    #[test]
    fn rename_keeps_the_extension_and_refuses_clashes() {
        let (_dir, roots, conn) = fixture();
        let src = roots.desktop.clone().unwrap().join("a.docx");
        std::fs::write(&src, b"a").unwrap();
        std::fs::write(roots.desktop.clone().unwrap().join("taken.docx"), b"t").unwrap();
        let renamed = rename_file(&conn, &roots, &src, "Grade 7 Quiz").unwrap();
        assert_eq!(renamed.file_name().unwrap().to_string_lossy(), "Grade 7 Quiz.docx");
        assert_eq!(rename_file(&conn, &roots, &renamed, "taken").unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(rename_file(&conn, &roots, &renamed, "bad:name").unwrap_err().code, ErrorCode::InvalidName);
    }

    #[test]
    fn outside_roots_is_refused_and_folders_are_created_once() {
        let (dir, roots, conn) = fixture();
        let outside = dir.path().join("x.txt");
        std::fs::write(&outside, b"x").unwrap();
        let dest = roots.documents.clone().unwrap();
        assert_eq!(move_file(&conn, &roots, &outside, &dest, OpKind::Move).unwrap_err().code, ErrorCode::OutsideRoots);
        let folder = create_folder(&conn, &roots, &dest, "Grade 7").unwrap();
        assert!(folder.is_dir());
        assert_eq!(create_folder(&conn, &roots, &dest, "Grade 7").unwrap_err().code, ErrorCode::AlreadyExists);
        let op = list(&conn, 1).unwrap().remove(0);
        undo(&conn, &roots, op.id).unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn a_manual_batch_undoes_together_and_keeps_its_folder() {
        let (_dir, roots, conn) = fixture();
        let downloads = roots.downloads.clone().unwrap();
        let a = downloads.join("Quiz A.docx");
        let b = downloads.join("Quiz B.docx");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        // A folder she made earlier, empty before the move.
        let dest = roots.documents.clone().unwrap().join("School Files").join("Quizzes");
        std::fs::create_dir_all(&dest).unwrap();
        let batch = new_batch("move");
        move_file_in_batch(&conn, &roots, &a, &dest, &batch).unwrap();
        move_file_in_batch(&conn, &roots, &b, &dest, &batch).unwrap();
        let logged = list(&conn, 10).unwrap();
        assert_eq!(logged.iter().filter(|o| o.batch.as_deref() == Some(batch.as_str())).count(), 2);

        let undone = undo_batch(&conn, &roots, &batch).unwrap();
        assert_eq!(undone.undone, 2);
        assert!(a.is_file() && b.is_file());
        assert!(dest.is_dir(), "a manual batch must never remove the folder it moved into");
    }

    #[test]
    fn batch_ids_are_unique_within_one_millisecond() {
        let first = new_batch("recycle");
        let second = new_batch("recycle");
        assert_ne!(first, second);
        assert!(first.starts_with("recycle-"));
    }
}
