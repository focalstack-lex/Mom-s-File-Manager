//! Every change to her files (spec 5.4 and 5.7): logged before it happens,
//! confined to the indexed roots, never overwriting, never deleting for good,
//! and undoable afterwards.

use crate::error::{ToteError, ErrorCode, Result, FILE_GONE, IN_USE, NAME_TAKEN};
use crate::index;
use crate::paths::{self, Roots};
use crate::sorter;
use crate::state::Placed;
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

/// A change to the ops table; a failure reads as not saved, not as a failed read.
fn write(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
    conn.execute(sql, params).map_err(ToteError::db_write)
}

fn record(conn: &Connection, kind: OpKind, src: &Path, dst: Option<&Path>, automatic: bool) -> Result<i64> {
    record_in(conn, kind, src, dst, automatic, None)
}

/// `batch` ties the changes of one action together so one Undo reverses them all.
fn record_in(conn: &Connection, kind: OpKind, src: &Path, dst: Option<&Path>, automatic: bool, batch: Option<&str>) -> Result<i64> {
    write(
        conn,
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
            write(conn, "UPDATE ops SET status = 'done', dst = ?2 WHERE id = ?1", params![id, dst.to_string_lossy().into_owned()])?;
        }
        Err(error) => {
            write(conn, "UPDATE ops SET status = 'failed', message = ?2 WHERE id = ?1", params![id, error.message])?;
        }
    }
    Ok(())
}

/// Rename when possible; copy, verify the size, then remove the source when
/// the destination is on another volume. A failed verification leaves the
/// original in place and removes the partial copy.
pub fn transfer(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(ToteError::new(ErrorCode::AlreadyExists, NAME_TAKEN).with_path(paths::display(dst)));
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
        return Err(ToteError::new(ErrorCode::Io, "The copy did not complete, so the file was left where it was. Try again in a moment.").with_path(paths::display(src)));
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
        return Err(ToteError::new(ErrorCode::NotFound, FILE_GONE).with_path(paths::display(&src)));
    }
    let name = src.file_name().ok_or_else(|| ToteError::new(ErrorCode::InvalidPath, "That file has no name Tote can use. Pick another file."))?;
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
        return Err(ToteError::new(ErrorCode::NotFound, FILE_GONE).with_path(paths::display(&src)));
    }
    paths::validate_name(new_stem)?;
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let parent = src.parent().ok_or_else(|| ToteError::new(ErrorCode::InvalidPath, "That file has no folder Tote can use. Pick another file."))?;
    let dst = parent.join(format!("{new_stem}{ext}"));
    if dst == src {
        return Ok(src);
    }
    if dst.exists() {
        return Err(ToteError::new(ErrorCode::AlreadyExists, NAME_TAKEN).with_path(paths::display(&dst)));
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
        return Err(ToteError::new(ErrorCode::AlreadyExists, "A folder with that name already exists. Pick another name.").with_path(paths::display(&dst)));
    }
    let id = record(conn, OpKind::NewFolder, &dst, None, false)?;
    let result = std::fs::create_dir(&dst).map(|_| dst.clone()).map_err(ToteError::from);
    finish(conn, id, &result)?;
    result
}

/// The longest path the Recycle Bin takes: with 260 or more UTF-16 units
/// Windows deletes the file for good instead, and says nothing when no
/// window is shown (audit T-01).
const RECYCLE_BIN_MAX_PATH: usize = 260;

/// How far the Recycle Bin's deletion time may sit from the moment Tote
/// recorded for Undo to accept an item as the file it recycled.
const RESTORE_TIME_SLACK_SECS: i64 = 30;

#[cfg(windows)]
fn path_units(path: &Path) -> usize {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().count()
}

#[cfg(not(windows))]
fn path_units(path: &Path) -> usize {
    path.as_os_str().len()
}

/// Only a fixed local drive has a Recycle Bin: a flash drive, a network
/// folder or a disc has none, and Windows deletes for good there.
#[cfg(windows)]
fn drive_has_recycle_bin(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let root = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => format!("{}:\\", char::from(letter)),
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!("\\\\{}\\{}\\", server.to_string_lossy(), share.to_string_lossy()),
            _ => return false,
        },
        _ => return false,
    };
    winsafe::GetDriveType(Some(&root)) == winsafe::co::DRIVE::FIXED
}

#[cfg(not(windows))]
fn drive_has_recycle_bin(_path: &Path) -> bool {
    true
}

/// Why `path` must not go to the Recycle Bin, as the sentence she sees, or
/// None when Windows can take it there.
fn recycle_refusal(path: &Path) -> Option<ToteError> {
    if path_units(path) >= RECYCLE_BIN_MAX_PATH {
        return Some(
            ToteError::new(ErrorCode::NoRecycleBin, "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.").with_path(paths::display(path)),
        );
    }
    if !drive_has_recycle_bin(path) {
        return Some(
            ToteError::new(ErrorCode::NoRecycleBin, "Files on a flash drive or a network folder cannot go to the Recycle Bin. Delete it in File Explorer if you are sure.")
                .with_path(paths::display(path)),
        );
    }
    None
}

/// The sentence a row carries when Windows took the file but the Recycle
/// Bin does not list it (the renderer leaves Show the file out for it).
const REMOVED_BY_WINDOWS: &str = "Windows removed it without putting it in the Recycle Bin, so it cannot be brought back.";

/// The sentence for a Recycle Bin call that failed; the library's text goes
/// to the log. The shell reports a file another program holds open as an
/// aborted operation rather than a sharing violation, so the file is also
/// tried for exclusive access to word that case as `inUse`.
fn recycle_failed(path: &Path, error: trash::Error) -> ToteError {
    log::warn!("recycle: {} could not be sent to the Recycle Bin ({error})", sorter::describe(path));
    log::debug!("recycle: the file was {}", path.display());
    let held_open = matches!(error, trash::Error::Os { code: 32 | 33, .. }) || (path.is_file() && !crate::stability::can_open_exclusively(path));
    let error = if held_open {
        ToteError::new(ErrorCode::InUse, IN_USE)
    } else {
        ToteError::new(ErrorCode::Io, "Windows could not send that file to the Recycle Bin. Try again in a moment.")
    };
    error.with_path(paths::display(path))
}

/// Sends files to the Recycle Bin, one logged operation per file. Several
/// files sent together share a batch, so Activity shows them as one row. A
/// file the Recycle Bin cannot take is refused and stays, its row failed
/// with the reason. Each row is finished as soon as its file is dealt with,
/// so a later target that fails leaves the earlier ones recorded; after the
/// deletes the Recycle Bin is read once, and a file it does not list is
/// marked failed as gone, never left as recycled.
pub fn recycle(conn: &Connection, roots: &Roots, targets: &[PathBuf]) -> Result<Vec<OpRow>> {
    let batch = (targets.len() > 1).then(|| new_batch("recycle"));
    let mut ids = Vec::new();
    // The files Windows took, to be checked against the Recycle Bin.
    let mut taken = Vec::new();
    for target in targets {
        let src = roots.confine(target)?;
        if !src.exists() {
            continue;
        }
        let id = record_in(conn, OpKind::Recycle, &src, None, false, batch.as_deref())?;
        let size = std::fs::metadata(&src).map(|m| m.len()).ok();
        let recycled_at = crate::db::now();
        let result = match recycle_refusal(&src) {
            Some(refusal) => Err(refusal),
            None => trash::delete(&src).map(|_| src.clone()).map_err(|e| recycle_failed(&src, e)),
        };
        finish(conn, id, &result)?;
        if result.is_ok() {
            write(conn, "UPDATE ops SET size = ?2, recycled_at = ?3 WHERE id = ?1", params![id, size.and_then(|s| i64::try_from(s).ok()), recycled_at])?;
            taken.push((id, src.clone(), recycled_at));
        }
        if !src.exists() {
            index::remove(conn, &src)?;
        }
        ids.push(id);
    }
    // A listing that fails was logged; the deletes themselves succeeded.
    if let Some(Ok(items)) = (!taken.is_empty()).then(bin_listing) {
        for (id, src, recycled_at) in &taken {
            let listed = items.iter().any(|item| is_same_original(item, src) && (item.time_deleted - *recycled_at).abs() <= RESTORE_TIME_SLACK_SECS);
            if !listed {
                log::warn!("recycle: Windows removed {} without putting it in the Recycle Bin", sorter::describe(src));
                log::debug!("recycle: the file was {}", src.display());
                write(conn, "UPDATE ops SET status = 'failed', message = ?2, size = NULL, recycled_at = NULL WHERE id = ?1", params![id, REMOVED_BY_WINDOWS])?;
            }
        }
    }
    let mut rows = Vec::new();
    for id in ids {
        if let Some(row) = get(conn, id)? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// One Recycle Bin item the way the shell reports it, kept apart from the
/// trash crate's type so the matching rules can be tested without a bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinItem {
    pub parent: String,
    pub name: String,
    pub time_deleted: i64,
    /// Asked for only when a recorded size must be matched: one shell call per item.
    pub size: Option<u64>,
}

#[cfg(any(windows, target_os = "linux"))]
impl From<&trash::TrashItem> for BinItem {
    fn from(item: &trash::TrashItem) -> Self {
        Self { parent: item.original_parent.to_string_lossy().into_owned(), name: item.name.to_string_lossy().into_owned(), time_deleted: item.time_deleted, size: None }
    }
}

/// The shell reports an item the way Explorer shows it, which can be
/// without the extension, so the match accepts the full name or the stem.
fn is_same_original(item: &BinItem, original: &Path) -> bool {
    let parent = original.parent().map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = original.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let stem = original.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let item_name = item.name.to_lowercase();
    item.parent.to_lowercase().trim_end_matches('\\') == parent.trim_end_matches('\\') && (item_name == name || item_name == stem)
}

/// Which of `items` Undo should restore for `original`. With the moment and
/// size Tote recorded at recycle time, only an item deleted within the slack
/// of that moment counts, the one with the same size first; a row from
/// before those were recorded takes the newest item with that path.
pub fn pick_restore(items: &[BinItem], original: &Path, recorded: Option<(i64, u64)>) -> Option<usize> {
    let same_path: Vec<usize> = (0..items.len()).filter(|&i| is_same_original(&items[i], original)).collect();
    let newest = |candidates: Vec<usize>| candidates.into_iter().max_by_key(|&i| items[i].time_deleted);
    let Some((at, size)) = recorded else { return newest(same_path) };
    let close: Vec<usize> = same_path.into_iter().filter(|&i| (items[i].time_deleted - at).abs() <= RESTORE_TIME_SLACK_SECS).collect();
    let same_size: Vec<usize> = close.iter().copied().filter(|&i| items[i].size == Some(size)).collect();
    if same_size.is_empty() {
        newest(close)
    } else {
        newest(same_size)
    }
}

/// What the Recycle Bin holds; a listing that fails is logged and returned
/// as the error, never mistaken for an empty bin.
#[cfg(any(windows, target_os = "linux"))]
fn bin_listing() -> Result<Vec<BinItem>> {
    let items = trash::os_limited::list().map_err(|e| {
        log::warn!("recycle: the Recycle Bin could not be listed ({e})");
        ToteError::new(ErrorCode::Io, "The Recycle Bin could not be checked. Open the Recycle Bin to see what is there.")
    })?;
    Ok(items.iter().map(BinItem::from).collect())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn bin_listing() -> Result<Vec<BinItem>> {
    Err(ToteError::new(ErrorCode::Unsupported, "The Recycle Bin is not available here. Delete the file in File Explorer if you are sure."))
}

/// The moment and size a Recycle Bin row recorded, when it is new enough to have them.
fn recorded_recycle(conn: &Connection, id: i64) -> Result<Option<(i64, u64)>> {
    let row: Option<(Option<i64>, Option<i64>)> = conn.query_row("SELECT recycled_at, size FROM ops WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    Ok(row.and_then(|(at, size)| Some((at?, u64::try_from(size?).ok()?))))
}

/// Tells the Downloads watcher that Tote is about to put a file at `path`
/// when that is in Downloads, so it is not filed again (audit T-06). Done
/// before the file moves, so the event cannot arrive first.
fn note_placed(roots: &Roots, placed: &Placed, path: &Path) {
    if roots.downloads.as_ref().is_some_and(|downloads| path.starts_with(downloads) || paths::resolve(downloads).is_ok_and(|d| path.starts_with(d))) {
        placed.note(path);
    }
}

/// True when an Undo once put a file back at `path`: the watcher's start-up
/// sweep leaves such a file alone, since the in-memory registry does not
/// survive a restart and a Windows shutdown skips the exit mark.
pub fn was_put_back(conn: &Connection, path: &Path) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT id FROM ops WHERE src = ?1 AND status = 'undone' LIMIT 1", [path.to_string_lossy().into_owned()], |r| r.get(0))
        .optional()?;
    Ok(found.is_some())
}

/// Reverses a completed operation. Moves and renames go back to their original
/// path (clash rule applied); a recycled file is restored from the Recycle Bin.
/// A file that comes back into Downloads is noted in `placed` first.
pub fn undo(conn: &Connection, roots: &Roots, placed: &Placed, id: i64) -> Result<()> {
    let row = get(conn, id)?.ok_or_else(|| ToteError::new(ErrorCode::NotFound, "That change is no longer in the list. Open Activity to see what is there."))?;
    if row.status != "done" {
        return Err(ToteError::validation("Only completed changes can be undone. Open Activity to check it."));
    }
    let src = PathBuf::from(&row.src);
    match row.kind.as_str() {
        "autoSort" | "move" | "rename" | "copy" => {
            let current = PathBuf::from(row.dst.as_deref().unwrap_or_default());
            let current = roots.confine(&current)?;
            if row.kind == "copy" {
                // A template copy is undone by recycling the copy; the master stays.
                if let Some(refusal) = recycle_refusal(&current) {
                    return Err(refusal);
                }
                trash::delete(&current).map_err(|e| recycle_failed(&current, e))?;
                index::remove(conn, &current)?;
            } else {
                let back = sorter::unique_path(&roots.confine(&src)?);
                note_placed(roots, placed, &back);
                transfer(&current, &back)?;
                index::relocate(conn, &current, &back)?;
            }
        }
        "newFolder" => {
            let folder = roots.confine(&src)?;
            if std::fs::read_dir(&folder).map(|mut d| d.next().is_some()).unwrap_or(false) {
                return Err(ToteError::validation("That folder has files in it now, so it stays. Empty it in File Explorer first if you want it gone."));
            }
            std::fs::remove_dir(&folder)?;
        }
        "recycle" => {
            let back = restore_from_recycle_bin(&src, recorded_recycle(conn, id)?, roots, placed)?;
            index::refresh_path(conn, &back)?;
        }
        other => {
            log::warn!("undo: change {id} is a {other}, which cannot be undone");
            return Err(ToteError::new(ErrorCode::Unsupported, "That kind of change cannot be undone. Move the file back by hand in File Explorer."));
        }
    }
    write(conn, "UPDATE ops SET status = 'undone' WHERE id = ?1", [id])?;
    Ok(())
}

fn not_in_recycle_bin() -> ToteError {
    ToteError::new(ErrorCode::NotInRecycleBin, "That file is not in the Recycle Bin any more. Open the Recycle Bin to check.")
}

/// Where Undo puts a recycled file back: its recorded path when that is
/// free, else the next free "Name (2).docx" beside it, the rule the Undo of
/// a move follows. A file that has since taken the path is never touched.
fn landing_for(original: &Path) -> PathBuf {
    sorter::unique_path(original)
}

/// The last check after a restore, on `landing` and nowhere else: a file of
/// another size than the recorded one is not the file she recycled, so
/// `send_back` returns it to the Recycle Bin where it came from. Nothing at
/// the landing means nothing came back, whatever Windows reported.
fn check_restored(landing: &Path, recorded: Option<(i64, u64)>, send_back: impl FnOnce(&Path)) -> Result<()> {
    let Ok(back) = std::fs::metadata(landing).map(|m| m.len()) else {
        log::warn!("restore: Windows reported the restore done but nothing came back ({})", sorter::describe(landing));
        log::debug!("restore: nothing is at {}", landing.display());
        return Err(not_in_recycle_bin());
    };
    if let Some((_, size)) = recorded {
        if back != size {
            log::warn!("restore: the item brought back ({}) has {back} bytes, not the recorded {size}; sending it back", sorter::describe(landing));
            log::debug!("restore: the item landed at {}", landing.display());
            send_back(landing);
            return Err(not_in_recycle_bin());
        }
    }
    Ok(())
}

/// Brings the item Tote recycled from `original` back (pick_restore chooses
/// it; restoring several at once is refused by Windows). The shell restores
/// an item under the name it is given, and the name it lists is the bare
/// stem when Explorer hides extensions, so it is given the full name at the
/// landing path instead; the file that lands there is then checked against
/// the recorded size. Returns where the file came back; a landing in
/// Downloads is noted in `placed` before the shell is asked.
#[cfg(any(windows, target_os = "linux"))]
fn restore_from_recycle_bin(original: &Path, recorded: Option<(i64, u64)>, roots: &Roots, placed: &Placed) -> Result<PathBuf> {
    use trash::os_limited;
    let items = os_limited::list().map_err(|error| {
        log::warn!("restore: the Recycle Bin could not be listed ({error})");
        ToteError::new(ErrorCode::Io, "Tote could not read the Recycle Bin. Open the Recycle Bin to bring the file back.")
    })?;
    let mut bin: Vec<BinItem> = items.iter().map(BinItem::from).collect();
    if recorded.is_some() {
        // Sizes cost one shell call each, so only the items with that path are asked.
        for (item, raw) in bin.iter_mut().zip(&items) {
            if is_same_original(item, original) {
                item.size = os_limited::metadata(raw).ok().and_then(|m| m.size.size());
            }
        }
    }
    let wanted = paths::display(original);
    let Some(chosen) = pick_restore(&bin, original, recorded) else {
        log::warn!("restore: nothing in the Recycle Bin is {} she recycled", sorter::describe(original));
        log::debug!("restore: the file was recycled from {wanted}");
        return Err(not_in_recycle_bin());
    };
    let mut latest = items.into_iter().nth(chosen).ok_or_else(not_in_recycle_bin)?;
    let landing = landing_for(original);
    let name = landing.file_name().ok_or_else(|| ToteError::new(ErrorCode::InvalidPath, "That file has no name Tote can use. Pick another file."))?;
    if landing != original {
        log::info!("restore: another file has taken the recorded path; the recycled one comes back beside it as (2)");
        log::debug!("restore: {wanted} is taken; landing at {}", landing.display());
    }
    log::info!("restoring {} from the Recycle Bin", sorter::describe(&landing));
    log::debug!("restoring {} from the Recycle Bin to {}", latest.id.to_string_lossy(), landing.display());
    latest.name = name.to_os_string();
    note_placed(roots, placed, &landing);
    os_limited::restore_all(vec![latest]).map_err(|error| {
        log::warn!("restore: Windows could not bring back {} she recycled ({error})", sorter::describe(original));
        log::debug!("restore: the file was recycled from {wanted}");
        ToteError::new(ErrorCode::Io, "Windows could not bring that file back. Open the Recycle Bin to restore it.")
    })?;
    check_restored(&landing, recorded, |path| {
        if let Err(error) = trash::delete(path) {
            log::warn!("restore: could not send it back ({error})");
        }
    })?;
    Ok(landing)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn restore_from_recycle_bin(_original: &Path, _recorded: Option<(i64, u64)>, _roots: &Roots, _placed: &Placed) -> Result<PathBuf> {
    Err(ToteError::new(ErrorCode::Unsupported, "Restoring from the Recycle Bin is not available here. Open the Recycle Bin to bring the file back."))
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
pub fn undo_batch(conn: &Connection, roots: &Roots, placed: &Placed, batch: &str) -> Result<BatchUndo> {
    let rows: Vec<OpRow> = {
        let mut stmt = conn.prepare("SELECT id, kind, src, dst, status, message, automatic, created_at, batch FROM ops WHERE batch = ?1 AND status = 'done' ORDER BY id DESC")?;
        let mapped = stmt.query_map([batch], map_op)?;
        mapped.collect::<std::result::Result<Vec<OpRow>, _>>()?
    };
    let mut undone = 0;
    let mut failed = Vec::new();
    let mut touched: Vec<PathBuf> = Vec::new();
    for row in rows {
        match undo(conn, roots, placed, row.id) {
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
    write(conn, "UPDATE ops SET status = 'failed', message = ?2 WHERE id = ?1", params![id, message])?;
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
        let placed = Placed::default();
        assert!(!was_put_back(&conn, &src).unwrap());
        undo(&conn, &roots, &placed, ops[0].id).unwrap();
        assert!(src.is_file());
        assert_eq!(get(&conn, ops[0].id).unwrap().unwrap().status, "undone");
        // The watcher is told before the file moves back, so it is not filed again (audit T-06).
        assert!(placed.covers(&src, std::time::Instant::now()), "the path Undo put back in Downloads is noted");
        assert!(was_put_back(&conn, &src).unwrap(), "and the start-up sweep will know it was put back");
        assert!(!was_put_back(&conn, &roots.downloads.clone().unwrap().join("Other.docx")).unwrap());
    }

    /// Undo of a move that did not land in Downloads notes nothing: the
    /// registry is only for the Downloads watcher.
    #[test]
    fn undo_outside_downloads_notes_nothing() {
        let (_dir, roots, conn) = fixture();
        let src = roots.desktop.clone().unwrap().join("Quiz.docx");
        std::fs::write(&src, b"q").unwrap();
        let dest = roots.documents.clone().unwrap().join("Forms");
        move_file(&conn, &roots, &src, &dest, OpKind::Move).unwrap();
        let placed = Placed::default();
        undo(&conn, &roots, &placed, list(&conn, 1).unwrap()[0].id).unwrap();
        assert!(src.is_file());
        assert!(!placed.covers(&src, std::time::Instant::now()));
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
        undo(&conn, &roots, &Placed::default(), op.id).unwrap();
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

        let placed = Placed::default();
        let undone = undo_batch(&conn, &roots, &placed, &batch).unwrap();
        assert_eq!(undone.undone, 2);
        assert!(a.is_file() && b.is_file());
        assert!(placed.covers(&a, std::time::Instant::now()) && placed.covers(&b, std::time::Instant::now()), "Undo all notes every file it puts back in Downloads");
        assert!(dest.is_dir(), "a manual batch must never remove the folder it moved into");
    }

    /// A file another program holds open (here with no sharing, the way
    /// Word holds a document) cannot be renamed or moved; she reads that it
    /// is open elsewhere, never the OS text, and Activity carries the same
    /// sentence (audit T-17).
    #[cfg(windows)]
    #[test]
    fn a_file_open_in_another_program_is_reported_as_in_use() {
        let (_dir, roots, conn) = fixture();
        let src = roots.desktop.clone().unwrap().join("Open.docx");
        std::fs::write(&src, b"open").unwrap();
        let _held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(0).open(&src).unwrap()
        };
        let error = rename_file(&conn, &roots, &src, "Renamed").unwrap_err();
        assert_eq!(error.code, ErrorCode::InUse, "{}", error.message);
        assert_eq!(error.message, IN_USE);
        let row = list(&conn, 1).unwrap().remove(0);
        assert_eq!(row.status, "failed");
        assert_eq!(row.message.as_deref(), Some(IN_USE), "Activity carries the same sentence");
        let error = move_file(&conn, &roots, &src, &roots.documents.clone().unwrap(), OpKind::Move).unwrap_err();
        assert_eq!(error.code, ErrorCode::InUse, "{}", error.message);
        assert!(src.is_file(), "the file stays where it was");
    }

    /// The Recycle Bin crate's own text never reaches her: a failed delete
    /// is one plain sentence, and a file held open reads as open elsewhere.
    #[test]
    fn a_failed_recycle_bin_call_is_worded_plainly() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Quiz.docx");
        std::fs::write(&file, b"q").unwrap();
        let aborted = || trash::Error::Unknown { description: "Some operations were aborted".into() };
        let error = recycle_failed(&file, aborted());
        assert_eq!(error.code, ErrorCode::Io);
        assert_eq!(error.message, "Windows could not send that file to the Recycle Bin. Try again in a moment.");
        assert_eq!(error.path.as_deref(), Some(paths::display(&file).as_str()));
        let sharing = recycle_failed(&file, trash::Error::Os { code: 32, description: "The process cannot access the file".into() });
        assert_eq!((sharing.code, sharing.message.as_str()), (ErrorCode::InUse, IN_USE));
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let _held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&file).unwrap();
            let error = recycle_failed(&file, aborted());
            assert_eq!((error.code, error.message.as_str()), (ErrorCode::InUse, IN_USE), "an aborted delete of a file held open");
        }
        for error in [recycle_failed(&file, aborted()), sharing] {
            assert!(!error.message.contains("Error during") && !error.message.contains("aborted"), "{}", error.message);
        }
    }

    /// A write that fails says not saved; a read that fails says not read.
    #[test]
    fn a_failed_write_says_not_saved_and_a_failed_read_says_not_read() {
        let (_dir, roots, conn) = fixture();
        let file = roots.downloads.clone().unwrap().join("Quiz.docx");
        std::fs::write(&file, b"q").unwrap();
        conn.pragma_update(None, "query_only", "ON").unwrap();
        let error = note_failure(&conn, &file, "x").unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (ErrorCode::Db, crate::error::DB_WRITE));
        assert!(list(&conn, 1).unwrap().is_empty(), "reads still work");
        conn.pragma_update(None, "query_only", "OFF").unwrap();
        conn.execute_batch("DROP TABLE ops").unwrap();
        let error = list(&conn, 1).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (ErrorCode::Db, crate::error::DB_READ));
    }

    #[test]
    fn batch_ids_are_unique_within_one_millisecond() {
        let first = new_batch("recycle");
        let second = new_batch("recycle");
        assert_ne!(first, second);
        assert!(first.starts_with("recycle-"));
    }

    #[test]
    fn pick_restore_prefers_the_recorded_moment_and_size() {
        let original = Path::new(r"C:\Users\Her\Desktop\Quiz.docx");
        let item = |name: &str, time_deleted: i64, size: Option<u64>| BinItem { parent: r"C:\Users\Her\Desktop".into(), name: name.into(), time_deleted, size };
        let items = vec![
            // The Tote recycle, shown without its extension because Explorer hides them.
            item("Quiz", 1_000, Some(100)),
            // Quiz.pdf, recycled from Explorer moments later.
            item("Quiz", 1_002, Some(2_000)),
            // A later file with the same name.
            item("Quiz.docx", 5_000, Some(100)),
            item("Other.docx", 1_000, Some(100)),
        ];
        assert_eq!(pick_restore(&items, original, Some((1_001, 100))), Some(0), "the same moment and size");
        assert_eq!(pick_restore(&items, original, Some((1_001, 777))), Some(1), "no size match: the newest within the slack");
        assert_eq!(pick_restore(&items, original, Some((9_000, 100))), None, "nothing deleted near the recorded moment");
        assert_eq!(pick_restore(&items, original, None), Some(2), "a row without a record takes the newest with that path");
        assert_eq!(pick_restore(&items, Path::new(r"C:\Users\Her\Desktop\Missing.docx"), None), None);
    }

    #[test]
    fn a_path_too_long_for_the_recycle_bin_is_refused_and_the_file_stays() {
        let (_dir, roots, conn) = fixture();
        let deep = roots.documents.clone().unwrap().join("A".repeat(120)).join("B".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        let file = deep.join("Deep Quiz.docx");
        std::fs::write(&file, b"deep").unwrap();
        assert!(path_units(&file) >= RECYCLE_BIN_MAX_PATH);
        let rows = recycle(&conn, &roots, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "failed");
        assert_eq!(rows[0].message.as_deref(), Some("This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first."));
        assert!(file.is_file(), "a refused file is left where it was");
        assert_eq!(undo(&conn, &roots, &Placed::default(), rows[0].id).unwrap_err().code, ErrorCode::Validation, "a refused row has nothing to undo");
    }

    /// With extensions hidden the shell lists the item as the bare stem; a
    /// file that has since taken the recorded path is never the one checked
    /// or sent back: the item lands beside it, and only the landing is judged.
    #[test]
    fn a_file_at_the_original_path_is_never_checked_or_sent_back() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("Quiz.docx");
        std::fs::write(&original, b"the file that took its place").unwrap();
        let item = BinItem { parent: dir.path().to_string_lossy().into_owned(), name: "Quiz".into(), time_deleted: 1_000, size: Some(100) };
        assert_eq!(pick_restore(std::slice::from_ref(&item), &original, Some((1_000, 100))), Some(0), "the bare stem still matches the row");
        let landing = landing_for(&original);
        assert_eq!(landing, dir.path().join("Quiz (2).docx"), "the item lands beside the file that took its place, with its extension");
        // What the shell put at the landing has another size: it goes back, the other file stays.
        std::fs::write(&landing, b"not the recorded 100 bytes").unwrap();
        let mut sent_back = Vec::new();
        let error = check_restored(&landing, Some((1_000, 100)), |path| sent_back.push(path.to_path_buf())).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotInRecycleBin);
        assert_eq!(sent_back, vec![landing.clone()]);
        assert_eq!(std::fs::read(&original).unwrap(), b"the file that took its place");
        // The recorded size passes; nothing at the landing is a failed restore.
        std::fs::write(&landing, [0u8; 100]).unwrap();
        check_restored(&landing, Some((1_000, 100)), |_| panic!("nothing to send back")).unwrap();
        std::fs::remove_file(&landing).unwrap();
        assert_eq!(check_restored(&landing, None, |_| panic!("nothing to send back")).unwrap_err().code, ErrorCode::NotInRecycleBin);
    }

    /// The real Recycle Bin on the system drive: the recycled file is listed
    /// there with its moment and size recorded, and Undo brings it back;
    /// when another file has taken its place by then, it comes back beside
    /// that file as "(2)" with its extension and the other file is left
    /// alone. Skipped where the temp folder is not on a fixed drive.
    #[cfg(windows)]
    #[test]
    fn a_recycled_file_is_found_in_the_recycle_bin_and_undo_brings_it_back() {
        let (_dir, roots, conn) = fixture();
        let file = roots.downloads.clone().unwrap().join("Tote recycle check.txt");
        std::fs::write(&file, b"recycle check").unwrap();
        if let Some(refusal) = recycle_refusal(&roots.confine(&file).unwrap()) {
            eprintln!("skipped: {}", refusal.message);
            return;
        }
        let rows = recycle(&conn, &roots, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rows[0].status, "done", "{:?}", rows[0].message);
        assert!(!file.exists());
        let (at, size) = recorded_recycle(&conn, rows[0].id).unwrap().unwrap();
        assert_eq!(size, 13);
        assert!((crate::db::now() - at).abs() < 60);
        let recycled = PathBuf::from(&rows[0].src);
        assert!(bin_listing().unwrap().iter().any(|item| is_same_original(item, &recycled)), "the item is in the Recycle Bin");
        let placed = Placed::default();
        undo(&conn, &roots, &placed, rows[0].id).unwrap();
        assert!(file.is_file(), "Undo brings it back");
        assert_eq!(get(&conn, rows[0].id).unwrap().unwrap().status, "undone");
        assert!(placed.covers(&file, std::time::Instant::now()), "a restore into Downloads is noted for the watcher");

        let rows = recycle(&conn, &roots, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rows[0].status, "done", "{:?}", rows[0].message);
        std::fs::write(&file, b"another file with that name").unwrap();
        undo(&conn, &roots, &placed, rows[0].id).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"another file with that name", "the file that took its place is never touched");
        let beside = roots.downloads.clone().unwrap().join("Tote recycle check (2).txt");
        assert_eq!(std::fs::read(&beside).unwrap(), b"recycle check", "the recycled file comes back beside it with its extension");
        assert!(placed.covers(&beside, std::time::Instant::now()), "the landing beside the other file is the path noted");
        assert!(index::by_path(&conn, &beside.to_string_lossy()).unwrap().is_some(), "the index knows where it landed");
        assert_eq!(get(&conn, rows[0].id).unwrap().unwrap().status, "undone");
    }

    /// A target that fails after others were sent (here one outside her
    /// folders) leaves their rows finished, so Activity shows them with Undo.
    /// Skipped where the temp folder is not on a fixed drive.
    #[cfg(windows)]
    #[test]
    fn a_later_target_that_fails_leaves_the_earlier_rows_finished() {
        let (dir, roots, conn) = fixture();
        let first = roots.downloads.clone().unwrap().join("Tote recycle batch.txt");
        std::fs::write(&first, b"first").unwrap();
        if let Some(refusal) = recycle_refusal(&roots.confine(&first).unwrap()) {
            eprintln!("skipped: {}", refusal.message);
            return;
        }
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, b"outside").unwrap();
        let error = recycle(&conn, &roots, &[first.clone(), outside.clone()]).unwrap_err();
        assert_eq!(error.code, ErrorCode::OutsideRoots);
        assert!(outside.is_file(), "the outside file was never touched");
        assert!(!first.exists());
        let rows = list(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1, "the first row is finished, not left pending: {rows:?}");
        assert_eq!(rows[0].status, "done");
        assert!(recorded_recycle(&conn, rows[0].id).unwrap().is_some(), "its moment and size are recorded");
        undo(&conn, &roots, &Placed::default(), rows[0].id).unwrap();
        assert!(first.is_file(), "and Undo works on it");
    }
}
