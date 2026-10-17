//! Every change to her files (spec 5.4 and 5.7): logged before it happens,
//! confined to the indexed roots, never overwriting, never deleting for good,
//! and undoable afterwards.

use crate::error::{ToteError, ErrorCode, Result, FILE_GONE, IN_USE, NAME_TAKEN};
use crate::index;
use crate::origin;
use crate::paths::{self, Roots};
use crate::sorter;
use crate::state::Db;
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
    /// A move or filing whose file carried the web stamp and arrived
    /// without it, because the drive it went to cannot hold one (a FAT32 or
    /// exFAT flash drive, audit T-43): Windows no longer marks it as from
    /// the internet.
    pub stamp_lost: bool,
}

/// The columns `map_op` reads, in its order.
const OP_COLUMNS: &str = "id, kind, src, dst, status, message, automatic, created_at, batch, stamp_lost";

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

/// The start of the name a copy between drives is written under until it
/// is whole (audit T-14). Find lists no name that starts with a dot, and
/// the start-up sweep (`resolve_pending`) removes what a closed Tote left.
pub const PARTIAL_PREFIX: &str = ".tote-partial-";

/// How many temporary names a copy tries before it gives up: another copy
/// to the same name at the same moment is the only reason a name is taken.
const PARTIAL_TRIES: usize = 8;

/// From this many UTF-16 units a direct Win32 call needs the `\\?\` form
/// of a path, which std adds on its own (the limit std uses).
#[cfg(windows)]
const WIN32_SHORT: usize = 248;

/// Moves `src` to `dst`, never over a file that is there. On one drive it
/// is a rename; between drives the file is copied under a temporary name
/// beside `dst` (`PARTIAL_PREFIX`), the copy takes the real name only once
/// its size matches, and the original is removed last (audit T-13, T-14).
/// A rename that fails for any other reason (most often the file is open in
/// another program) returns that reason and copies nothing. When the
/// original cannot be removed after the copy, the copy is taken back out,
/// so she never has two, and the reason is returned. Runs with no database
/// lock held (audit T-12).
pub fn transfer(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(ToteError::new(ErrorCode::AlreadyExists, NAME_TAKEN).with_path(paths::display(dst)));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !copy_moves_for_test() {
        match place(src, dst) {
            Ok(()) => return Ok(()),
            Err(error) if crosses_drives(&error) => {}
            Err(error) => return Err(io_failed(error, src, dst)),
        }
    }
    copy_across(src, dst)
}

/// The copy between drives: under a temporary name, checked, given the
/// real name, and only then is the original removed.
fn copy_across(src: &Path, dst: &Path) -> Result<()> {
    let expected = std::fs::metadata(src).map_err(|error| io_failed(error, src, dst))?.len();
    let partial = copy_to_partial(src, dst)?;
    let copied = std::fs::metadata(&partial).map(|m| m.len()).ok();
    if copied != Some(expected) {
        log::warn!("transfer: the copy of {} came out at {copied:?} bytes, not {expected}; the original stays", sorter::describe(src));
        discard(&partial);
        return Err(ToteError::new(ErrorCode::Io, "The copy did not complete, so the file was left where it was. Try again in a moment.").with_path(paths::display(src)));
    }
    if let Err(error) = place(&partial, dst) {
        discard(&partial);
        return Err(io_failed(error, src, dst));
    }
    if let Err(error) = std::fs::remove_file(src) {
        // Most often another program holds the original open (Word, a PDF
        // reader): the copy is taken back out, so she never has two.
        log::warn!("transfer: the original {} could not be removed after the copy, so the copy is taken back out", sorter::describe(src));
        log::debug!("transfer: {} stays where it was", src.display());
        if !discard(dst) {
            log::error!("transfer: a second copy of {} stayed beside the original", sorter::describe(src));
        }
        return Err(io_failed(error, src, dst));
    }
    Ok(())
}

/// Copies `src` beside `dst` under a temporary name nothing has yet and
/// returns that name; a name another copy took first is passed over, never
/// written into.
fn copy_to_partial(src: &Path, dst: &Path) -> Result<PathBuf> {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let wanted = dst.with_file_name(format!("{PARTIAL_PREFIX}{name}"));
    for _ in 0..PARTIAL_TRIES {
        let partial = sorter::unique_path(&wanted);
        match copy_new(src, &partial) {
            Ok(()) => return Ok(partial),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                discard(&partial);
                return Err(io_failed(error, src, dst));
            }
        }
    }
    log::warn!("transfer: no free temporary name beside the destination of {}", sorter::describe(src));
    Err(ToteError::new(ErrorCode::Io, "Tote could not finish that change. Try again in a moment.").with_path(paths::display(src)))
}

/// An io error from a transfer as the sentence she reads, with the path it
/// is about: the destination when its name was taken, else the file.
fn io_failed(error: std::io::Error, src: &Path, dst: &Path) -> ToteError {
    let at = if error.kind() == std::io::ErrorKind::AlreadyExists { dst } else { src };
    ToteError::from(error).with_path(paths::display(at))
}

/// Removes a copy Tote made itself (a temporary one, or a whole one whose
/// original could not be removed). Returns false, logged, when it stays.
fn discard(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            log::warn!("transfer: a copy Tote made could not be removed ({error})");
            log::debug!("transfer: the copy is {}", path.display());
            false
        }
    }
}

/// A rename refused because the two paths are on different drives:
/// ERROR_NOT_SAME_DEVICE on Windows, EXDEV elsewhere.
fn crosses_drives(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(if cfg!(windows) { 17 } else { 18 })
}

/// Test mode only (TOTE_ROOTS set): TOTE_TEST_COPY_MOVES sends every move
/// through the copy between drives, so the native check can time a search
/// during a long copy and close Tote in the middle of one on a laptop with
/// one drive.
fn copy_moves_for_test() -> bool {
    Roots::test_mode() && std::env::var_os("TOTE_TEST_COPY_MOVES").is_some()
}

/// Renames `from` to `to` only while nothing is at `to`; `std::fs::rename`
/// would replace a file that is there. Between drives it fails with
/// ERROR_NOT_SAME_DEVICE (`crosses_drives`).
#[cfg(windows)]
fn place(from: &Path, to: &Path) -> std::io::Result<()> {
    winsafe::MoveFileEx(&win32_path(from), Some(&win32_path(to)), winsafe::co::MOVEFILE::default()).map_err(os_error)
}

/// Copies `from` to `to` only while nothing is at `to`, with its streams,
/// so the web stamp goes along where the drive can hold it.
#[cfg(windows)]
fn copy_new(from: &Path, to: &Path) -> std::io::Result<()> {
    winsafe::CopyFile(&win32_path(from), &win32_path(to), true).map_err(os_error)
}

#[cfg(windows)]
fn os_error(error: winsafe::co::ERROR) -> std::io::Error {
    std::io::Error::from_raw_os_error(error.raw() as i32)
}

/// The spelling a direct Win32 call needs: from `WIN32_SHORT` units on, the
/// `\\?\` form of the full path.
#[cfg(windows)]
fn win32_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if path_units(path) < WIN32_SHORT || text.starts_with(r"\\?\") {
        return text.into_owned();
    }
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let full = full.to_string_lossy();
    match full.strip_prefix(r"\\") {
        Some(share) => format!(r"\\?\UNC\{share}"),
        None => format!(r"\\?\{full}"),
    }
}

#[cfg(not(windows))]
fn place(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.symlink_metadata().is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from, to)
}

#[cfg(not(windows))]
fn copy_new(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.symlink_metadata().is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::copy(from, to).map(|_| ())
}

/// Moves one file into `dst_dir`, clash rule applied. Returns the new path.
/// The lock is held to record the change and to finish it, never while the
/// file moves (audit T-12).
pub fn move_file(db: &impl Db, roots: &Roots, src: &Path, dst_dir: &Path, kind: OpKind) -> Result<PathBuf> {
    move_inner(db, roots, src, dst_dir, kind, None)
}

/// A move that belongs to an Organize run (spec 5.11) or to a Move to of several files.
pub fn move_file_in_batch(db: &impl Db, roots: &Roots, src: &Path, dst_dir: &Path, batch: &str) -> Result<PathBuf> {
    move_inner(db, roots, src, dst_dir, OpKind::Move, Some(batch))
}

/// A filing that belongs to one Move to Tote of several files (filing.rs).
pub fn file_in_batch(db: &impl Db, roots: &Roots, src: &Path, dst_dir: &Path, batch: &str) -> Result<PathBuf> {
    move_inner(db, roots, src, dst_dir, OpKind::AutoSort, Some(batch))
}

fn move_inner(db: &impl Db, roots: &Roots, src: &Path, dst_dir: &Path, kind: OpKind, batch: Option<&str>) -> Result<PathBuf> {
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
    // Folders this move is about to create, so Undo all removes those and
    // never one she had before (audit T-45).
    let creating = if batch.is_some() { missing_folders(&dst_dir) } else { Vec::new() };
    let id = record_in(&*db.hold()?, kind, &src, Some(&dst), kind == OpKind::AutoSort, batch)?;
    let stamped = origin::read_zone(&src).is_some();
    let result = transfer(&src, &dst).map(|()| dst.clone());
    let stamp_lost = stamped && result.is_ok() && origin::read_zone(&dst).is_none();
    let (dst, known) = {
        let conn = db.hold()?;
        finish(&conn, id, &result)?;
        let dst = result?;
        if let Some(batch) = batch {
            note_batch_folders(&conn, batch, &creating);
        }
        if stamp_lost {
            note_stamp_lost(&conn, id, &dst)?;
        }
        let known = index::relocate_row(&conn, &src, &dst)?;
        (dst, known)
    };
    if !known {
        index::refresh_file(db, &dst)?;
    }
    Ok(dst)
}

/// The settings row that lists the folders a batch created, as a JSON list.
const BATCH_FOLDERS_PREFIX: &str = "batch_folders:";

/// `dir` and those of its parents that do not exist yet, deepest first.
fn missing_folders(dir: &Path) -> Vec<PathBuf> {
    dir.ancestors().take_while(|folder| !folder.exists()).map(Path::to_path_buf).collect()
}

/// Adds `created` to the folders `batch` made. A failure to save the list
/// is logged and the move still counts: the cost is only an empty folder
/// left after Undo all.
fn note_batch_folders(conn: &Connection, batch: &str, created: &[PathBuf]) {
    if created.is_empty() {
        return;
    }
    let saved = (|| -> Result<()> {
        let mut folders = batch_folders(conn, batch)?;
        for folder in created {
            let text = folder.to_string_lossy().into_owned();
            if !folders.contains(&text) {
                folders.push(text);
            }
        }
        let json = serde_json::to_string(&folders).map_err(ToteError::db_write)?;
        crate::db::set_setting(conn, &format!("{BATCH_FOLDERS_PREFIX}{batch}"), &json).map_err(ToteError::as_write)
    })();
    if let Err(error) = saved {
        log::warn!("batch {batch}: the folders it created were not saved ({}); Undo all will leave them", error.message);
    }
}

/// The folders `batch` created, as saved by `note_batch_folders`.
fn batch_folders(conn: &Connection, batch: &str) -> Result<Vec<String>> {
    let Some(json) = crate::db::get_setting(conn, &format!("{BATCH_FOLDERS_PREFIX}{batch}"))? else { return Ok(Vec::new()) };
    Ok(serde_json::from_str(&json).unwrap_or_else(|error| {
        log::warn!("batch {batch}: the list of folders it created could not be read ({error})");
        Vec::new()
    }))
}

/// Logs and records that a moved file arrived without its web stamp (audit T-43).
fn note_stamp_lost(conn: &Connection, id: i64, dst: &Path) -> Result<()> {
    log::info!("moved {} to a drive that cannot keep the web stamp; Windows will no longer mark it as from the internet", sorter::describe(dst));
    log::debug!("the file without its stamp is {}", dst.display());
    write(conn, "UPDATE ops SET stamp_lost = 1 WHERE id = ?1", [id]).map(|_| ())
}

/// Renames a file in place, keeping its extension locked. A name taken by
/// another file is refused with `NAME_TAKEN`, unless `pick_free` asks for
/// the next free name ("Name (2).docx"), which she agreed to after that
/// refusal. A change of capital letters only is a rename too (audit T-28).
pub fn rename_file(conn: &Connection, roots: &Roots, src: &Path, new_stem: &str, pick_free: bool) -> Result<PathBuf> {
    let src = roots.confine(src)?;
    if !src.is_file() {
        return Err(ToteError::new(ErrorCode::NotFound, FILE_GONE).with_path(paths::display(&src)));
    }
    paths::validate_name(new_stem)?;
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let parent = src.parent().ok_or_else(|| ToteError::new(ErrorCode::InvalidPath, "That file has no folder Tote can use. Pick another file."))?;
    let mut dst = parent.join(format!("{new_stem}{ext}"));
    if dst == src {
        return Ok(src);
    }
    if name_taken(&src, &dst) {
        if !pick_free {
            return Err(ToteError::new(ErrorCode::AlreadyExists, NAME_TAKEN).with_path(paths::display(&dst)));
        }
        dst = sorter::unique_path(&dst);
    }
    let id = record(conn, OpKind::Rename, &src, Some(&dst), false)?;
    let result = std::fs::rename(&src, &dst).map(|_| dst.clone()).map_err(ToteError::from);
    finish(conn, id, &result)?;
    let dst = result?;
    index::relocate(conn, &src, &dst)?;
    Ok(dst)
}

/// Another entry already has the name `dst`. Windows compares names
/// without case, so for a change of capitals only ("lesson plan.docx" to
/// "Lesson Plan.docx") `exists()` answers for the file itself; the name is
/// taken then only when the folder holds an entry spelled exactly `dst`.
fn name_taken(src: &Path, dst: &Path) -> bool {
    if !dst.exists() {
        return false;
    }
    let (Some(from), Some(to)) = (src.file_name(), dst.file_name()) else { return true };
    if from.to_string_lossy().to_lowercase() != to.to_string_lossy().to_lowercase() || src.parent() != dst.parent() {
        return true;
    }
    let Some(parent) = dst.parent() else { return true };
    match std::fs::read_dir(parent) {
        Ok(entries) => entries.flatten().any(|e| e.file_name() == to),
        Err(error) => {
            log::warn!("rename: could not read the folder to check a change of capitals ({error})");
            true
        }
    }
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

/// Reverses a completed operation. Moves and renames go back to their original
/// path (clash rule applied); a recycled file is restored from the Recycle Bin.
/// The row then records the moment and where the file landed (`undone_at`,
/// `undone_to`); Undo all and a restore come through here, so they record it
/// too. The lock is held only to read and write the row and the index, never
/// while a file moves (audit T-12); the landing a move back is headed for is
/// written to the row first (`undone_to` on a row still done).
pub fn undo(db: &impl Db, roots: &Roots, id: i64) -> Result<()> {
    let row = get(&*db.hold()?, id)?.ok_or_else(|| ToteError::new(ErrorCode::NotFound, "That change is no longer in the list. Open Activity to see what is there."))?;
    if row.status != "done" {
        return Err(ToteError::validation("Only completed changes can be undone. Open Activity to check it."));
    }
    let src = PathBuf::from(&row.src);
    let landing = match row.kind.as_str() {
        "autoSort" | "move" | "rename" | "copy" => {
            let current = PathBuf::from(row.dst.as_deref().unwrap_or_default());
            let current = roots.confine(&current)?;
            if row.kind == "copy" {
                // A template copy is undone by recycling the copy; the master stays.
                if let Some(refusal) = recycle_refusal(&current) {
                    return Err(refusal);
                }
                trash::delete(&current).map_err(|e| recycle_failed(&current, e))?;
                index::remove(&*db.hold()?, &current)?;
                None
            } else {
                let back = sorter::unique_path(&roots.confine(&src)?);
                write(&*db.hold()?, "UPDATE ops SET undone_to = ?2 WHERE id = ?1", params![id, back.to_string_lossy().into_owned()])?;
                transfer(&current, &back)?;
                index::relocate_unlocked(db, &current, &back)?;
                Some(back)
            }
        }
        "newFolder" => {
            let folder = roots.confine(&src)?;
            if std::fs::read_dir(&folder).map(|mut d| d.next().is_some()).unwrap_or(false) {
                return Err(ToteError::validation("That folder has files in it now, so it stays. Empty it in File Explorer first if you want it gone."));
            }
            std::fs::remove_dir(&folder)?;
            None
        }
        "recycle" => {
            let recorded = recorded_recycle(&*db.hold()?, id)?;
            let back = restore_from_recycle_bin(&src, recorded)?;
            index::refresh_file(db, &back)?;
            Some(back)
        }
        other => {
            log::warn!("undo: change {id} is a {other}, which cannot be undone");
            return Err(ToteError::new(ErrorCode::Unsupported, "That kind of change cannot be undone. Move the file back by hand in File Explorer."));
        }
    };
    write(
        &*db.hold()?,
        "UPDATE ops SET status = 'undone', undone_at = ?2, undone_to = ?3 WHERE id = ?1",
        params![id, crate::db::now(), landing.map(|back| back.to_string_lossy().into_owned())],
    )?;
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
/// the recorded size. Returns where the file came back.
#[cfg(any(windows, target_os = "linux"))]
fn restore_from_recycle_bin(original: &Path, recorded: Option<(i64, u64)>) -> Result<PathBuf> {
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
fn restore_from_recycle_bin(_original: &Path, _recorded: Option<(i64, u64)>) -> Result<PathBuf> {
    Err(ToteError::new(ErrorCode::Unsupported, "Restoring from the Recycle Bin is not available here. Open the Recycle Bin to bring the file back."))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUndo {
    pub undone: usize,
    /// Why a file could not go back, so the toast and Activity name the
    /// reason instead of guessing (a file open in another program).
    pub failed: Vec<UndoFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoFailure {
    pub name: String,
    pub message: String,
}

/// Reverses every completed change of one batch, newest first, then
/// removes the folders the batch itself created (`note_batch_folders`)
/// that are now empty: never a folder that existed before, even an empty
/// one she made, never one that still holds anything, never School Files
/// itself (audit T-45). A batch recorded before that list existed removes
/// no folder.
pub fn undo_batch(db: &impl Db, roots: &Roots, batch: &str) -> Result<BatchUndo> {
    let rows: Vec<OpRow> = {
        let conn = db.hold()?;
        let mut stmt = conn.prepare(&format!("SELECT {OP_COLUMNS} FROM ops WHERE batch = ?1 AND status = 'done' ORDER BY id DESC"))?;
        let mapped = stmt.query_map([batch], map_op)?;
        mapped.collect::<std::result::Result<Vec<OpRow>, _>>()?
    };
    let mut undone = 0;
    let mut failed = Vec::new();
    for row in rows {
        match undo(db, roots, row.id) {
            Ok(()) => undone += 1,
            Err(error) => {
                log::warn!("undo batch {batch}, op {}: {}", row.id, error.message);
                failed.push(UndoFailure {
                    name: Path::new(&row.src).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    message: error.message,
                });
            }
        }
    }
    let mut created: Vec<PathBuf> = batch_folders(&*db.hold()?, batch)?.into_iter().map(PathBuf::from).collect();
    created.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    let school = roots.school_files().ok();
    for folder in created {
        if school.as_deref().is_some_and(|s| paths::same_path(s, &folder)) || roots.confine(&folder).is_err() {
            continue;
        }
        let empty = std::fs::read_dir(&folder).map(|mut d| d.next().is_none()).unwrap_or(false);
        if empty {
            if let Err(error) = std::fs::remove_dir(&folder) {
                log::warn!("undo batch {batch}: an empty folder it created stays ({error})");
            }
        }
    }
    if failed.is_empty() {
        // Everything is back, so the list has done its job.
        if let Err(error) = write(&*db.hold()?, "DELETE FROM settings WHERE key = ?1", [format!("{BATCH_FOLDERS_PREFIX}{batch}")]) {
            log::warn!("undo batch {batch}: the list of folders it created stays ({})", error.message);
        }
    }
    Ok(BatchUndo { undone, failed })
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<OpRow>> {
    let mut stmt = conn.prepare(&format!("SELECT {OP_COLUMNS} FROM ops WHERE id = ?1"))?;
    Ok(stmt.query_row([id], map_op).optional()?)
}

/// The newest `limit` entries of Activity, newest first, where all the
/// rows of one batch count as one entry and come back together, so a
/// 300-file Organize run is never cut short (audit T-45).
pub fn list(conn: &Connection, limit: usize) -> Result<Vec<OpRow>> {
    let mut stmt = conn.prepare(&format!(
        "WITH entries AS (SELECT COALESCE(batch, '#' || id) AS entry FROM ops WHERE status <> 'pending' GROUP BY entry ORDER BY MAX(id) DESC LIMIT ?1) \
         SELECT {OP_COLUMNS} FROM ops WHERE status <> 'pending' AND COALESCE(batch, '#' || id) IN (SELECT entry FROM entries) ORDER BY id DESC"
    ))?;
    let rows = stmt.query_map([limit as i64], map_op)?;
    Ok(rows.collect::<std::result::Result<Vec<OpRow>, _>>()?)
}

/// The note a change Tote was closed in the middle of carries in Activity.
pub const CLOSED_BEFORE_DONE: &str = "Tote was closed before this finished. Try it again.";

/// Settles what a closed or crashed Tote left half done, at start before
/// anything else runs (audit T-14, T-48). A pending row whose change is
/// complete on disk (the file at its new place and gone from the old one, a
/// template copy that is there, a folder that is there, a file gone to the
/// Recycle Bin) is done, and a moved file's index row follows it; any other
/// pending row failed with `CLOSED_BEFORE_DONE`, so Activity shows it
/// instead of hiding it. A copy left under a temporary name
/// (`PARTIAL_PREFIX`) beside a pending move's destination or beside the
/// landing an Undo was writing (`undone_to` on a row still done) is
/// removed: the original is removed only after the copy has its real name,
/// so a temporary copy is never the only one. A copy that already had its
/// real name while the original was still there (Tote closed between the
/// two steps) is a second copy of the same size and is removed too, so she
/// never has two. An Undo that finished on disk is recorded as undone. One
/// row that cannot be settled is logged and the rest still are. Returns how
/// many rows were.
pub fn resolve_pending(conn: &Connection) -> Result<usize> {
    let pending: Vec<(i64, String, String, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT id, kind, src, dst FROM ops WHERE status = 'pending'")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    // The folders where a copy may have been cut short.
    let mut folders: Vec<PathBuf> = Vec::new();
    let mut settled = 0;
    for (id, kind, src, dst) in &pending {
        let (src, dst) = (Path::new(src), dst.as_deref().map(Path::new));
        if let Some(parent) = dst.and_then(Path::parent) {
            folders.push(parent.to_path_buf());
        }
        let moved = matches!(kind.as_str(), "autoSort" | "move" | "rename");
        let finished = match kind.as_str() {
            "autoSort" | "move" | "rename" => dst.is_some_and(|d| d.exists()) && !src.exists(),
            "copy" => dst.is_some_and(|d| d.exists()),
            "newFolder" => src.is_dir(),
            "recycle" => !src.exists(),
            _ => false,
        };
        let outcome = if finished {
            write(conn, "UPDATE ops SET status = 'done' WHERE id = ?1", [id]).and_then(|_| match dst.filter(|_| moved) {
                Some(dst) => index::relocate_row(conn, src, dst).map(|_| ()),
                None => Ok(()),
            })
        } else {
            if matches!(kind.as_str(), "autoSort" | "move") {
                if let Some(dst) = dst.filter(|d| same_size_copy(src, d)) {
                    // The copy between drives had its name, the original was not yet removed.
                    discard(dst);
                }
            }
            write(conn, "UPDATE ops SET status = 'failed', message = ?2 WHERE id = ?1", params![id, CLOSED_BEFORE_DONE]).map(|_| ())
        };
        match outcome {
            Ok(()) => settled += 1,
            Err(error) => log::warn!("start: change {id} Tote was closed in the middle of could not be settled: {error}"),
        }
    }
    // Undos Tote was closed in the middle of: the landing was written to the row before the file moved back.
    let undoing: Vec<(i64, Option<String>, String)> = {
        let mut stmt = conn.prepare("SELECT id, dst, undone_to FROM ops WHERE status = 'done' AND undone_to IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (id, current, landing) in &undoing {
        let (current, landing) = (current.as_deref().map(Path::new), Path::new(landing));
        if let Some(parent) = landing.parent() {
            folders.push(parent.to_path_buf());
        }
        let back = landing.is_file() && !current.is_some_and(Path::exists);
        let outcome = if back {
            // The file is back where it was: the Undo is done, and its index row follows it.
            write(conn, "UPDATE ops SET status = 'undone', undone_at = ?2 WHERE id = ?1", params![id, crate::db::now()]).and_then(|_| match current {
                Some(current) => index::relocate_row(conn, current, landing).map(|_| ()),
                None => Ok(()),
            })
        } else {
            if let Some(current) = current.filter(|c| same_size_copy(c, landing)) {
                log::debug!("start: a copy an Undo had made stays beside {}", current.display());
                discard(landing);
            }
            write(conn, "UPDATE ops SET undone_to = NULL WHERE id = ?1", [id]).map(|_| ())
        };
        match outcome {
            Ok(()) => settled += usize::from(back),
            Err(error) => log::warn!("start: the Undo of change {id} Tote was closed in the middle of could not be settled: {error}"),
        }
    }
    folders.sort();
    folders.dedup();
    for folder in &folders {
        remove_partials(folder);
    }
    if settled > 0 {
        log::info!("start: settled {settled} changes Tote was closed in the middle of");
    }
    Ok(settled)
}

/// Whether `copy` is a whole second copy of `original`: both are files of
/// the same size. A move's destination and an Undo's landing were free when
/// the row was written, and nothing but the copy writes there, so a file of
/// the same size there while the original stays is the copy.
fn same_size_copy(original: &Path, copy: &Path) -> bool {
    match (std::fs::metadata(original), std::fs::metadata(copy)) {
        (Ok(a), Ok(b)) => a.is_file() && b.is_file() && a.len() == b.len(),
        _ => false,
    }
}

/// Removes the temporary copies (`PARTIAL_PREFIX`) a closed Tote left in `folder`.
fn remove_partials(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let partial = entry.file_name().to_string_lossy().starts_with(PARTIAL_PREFIX) && entry.file_type().is_ok_and(|t| t.is_file());
        if partial && discard(&entry.path()) {
            log::info!("start: removed a copy Tote was closed in the middle of");
            log::debug!("start: the copy was {}", entry.path().display());
        }
    }
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
        stamp_lost: row.get::<_, i64>(9)? != 0,
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
        let created = index::entry_for(&src).unwrap().created;
        let moved = move_file(&conn, &roots, &src, &dest, OpKind::Move).unwrap();
        assert!(moved.is_file());
        assert!(!src.exists());
        assert!(index::by_path(&conn, &moved.to_string_lossy()).unwrap().is_some());
        let ops = list(&conn, 10).unwrap();
        assert_eq!(ops[0].status, "done");
        let before_undo = db::now();
        undo(&conn, &roots, ops[0].id).unwrap();
        assert!(src.is_file());
        assert_eq!(get(&conn, ops[0].id).unwrap().unwrap().status, "undone");
        let (undone_at, undone_to) = undone(&conn, ops[0].id);
        let undone_at = undone_at.expect("the moment of the Undo is recorded");
        assert!(undone_at >= before_undo && undone_at <= db::now());
        assert_eq!(undone_to, Some(src.to_string_lossy().into_owned()), "and where the file landed");
        assert_eq!(index::entry_for(&src).unwrap().created, created, "a same-drive move keeps the created time");
    }

    /// The moment and the landing an undone row recorded.
    fn undone(conn: &Connection, id: i64) -> (Option<i64>, Option<String>) {
        conn.query_row("SELECT undone_at, undone_to FROM ops WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
    }

    /// Undo beside a file that has since taken the name lands at
    /// "Name (2)", leaves the other file untouched, and the row names the
    /// real landing.
    #[test]
    fn an_undo_beside_a_namesake_lands_beside_it() {
        let (_dir, roots, conn) = fixture();
        let downloads = roots.downloads.clone().unwrap();
        let school = roots.documents.clone().unwrap().join("School Files");
        let src = downloads.join("Quiz.docx");
        std::fs::write(&src, b"first").unwrap();
        move_file(&conn, &roots, &src, &school.join("Word"), OpKind::AutoSort).unwrap();
        std::fs::write(&src, b"second download").unwrap();
        let row = list(&conn, 1).unwrap().remove(0);
        undo(&conn, &roots, row.id).unwrap();
        let landing = downloads.join("Quiz (2).docx");
        assert!(landing.is_file(), "the first file came back beside the namesake");
        assert_eq!(std::fs::read(&src).unwrap(), b"second download", "which is untouched");
        let (_, undone_to) = undone(&conn, row.id);
        assert_eq!(undone_to, Some(landing.to_string_lossy().into_owned()), "the row names the real landing");
    }

    #[test]
    fn rename_keeps_the_extension_and_refuses_clashes() {
        let (_dir, roots, conn) = fixture();
        let src = roots.desktop.clone().unwrap().join("a.docx");
        std::fs::write(&src, b"a").unwrap();
        std::fs::write(roots.desktop.clone().unwrap().join("taken.docx"), b"t").unwrap();
        let renamed = rename_file(&conn, &roots, &src, "Grade 7 Quiz", false).unwrap();
        assert_eq!(renamed.file_name().unwrap().to_string_lossy(), "Grade 7 Quiz.docx");
        assert_eq!(rename_file(&conn, &roots, &renamed, "taken", false).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(rename_file(&conn, &roots, &renamed, "bad:name", false).unwrap_err().code, ErrorCode::InvalidName);
    }

    /// Fixing the capitals of a name is a rename, not a clash (audit T-28).
    #[test]
    fn a_change_of_capitals_only_is_a_rename() {
        let (_dir, roots, conn) = fixture();
        let src = roots.desktop.clone().unwrap().join("lesson plan.docx");
        std::fs::write(&src, b"plan").unwrap();
        let mut entries = Vec::new();
        index::walk(&roots.confine(roots.desktop.as_ref().unwrap()).unwrap(), &mut entries);
        for e in &entries {
            index::upsert(&conn, e).unwrap();
        }
        let renamed = rename_file(&conn, &roots, &src, "Lesson Plan", false).unwrap();
        assert_eq!(renamed.file_name().unwrap().to_string_lossy(), "Lesson Plan.docx");
        let names: Vec<String> = std::fs::read_dir(roots.desktop.as_ref().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Lesson Plan.docx".to_string()], "one file, with the new capitals");
        assert_eq!(std::fs::read(&renamed).unwrap(), b"plan");
        assert!(index::by_path(&conn, &renamed.to_string_lossy()).unwrap().is_some(), "the index follows the new spelling");
    }

    /// After she agrees, a taken name becomes the next free one (audit T-28).
    #[test]
    fn pick_free_takes_the_next_free_name() {
        let (_dir, roots, conn) = fixture();
        let desktop = roots.desktop.clone().unwrap();
        let src = desktop.join("draft.docx");
        std::fs::write(&src, b"draft").unwrap();
        std::fs::write(desktop.join("Grade 7 Quiz.docx"), b"theirs").unwrap();
        assert_eq!(rename_file(&conn, &roots, &src, "Grade 7 Quiz", false).unwrap_err().code, ErrorCode::AlreadyExists);
        let renamed = rename_file(&conn, &roots, &src, "Grade 7 Quiz", true).unwrap();
        assert_eq!(renamed.file_name().unwrap().to_string_lossy(), "Grade 7 Quiz (2).docx");
        assert_eq!(std::fs::read(desktop.join("Grade 7 Quiz.docx")).unwrap(), b"theirs", "the other file is untouched");
        assert_eq!(std::fs::read(&renamed).unwrap(), b"draft");
    }

    /// Activity counts a batch as one entry and returns all of its rows,
    /// however many there are (audit T-45).
    #[test]
    fn activity_returns_whole_batches() {
        let (_dir, _roots, conn) = fixture();
        let insert = |src: String, batch: Option<&str>| {
            conn.execute("INSERT INTO ops(kind, src, dst, status, automatic, created_at, batch) VALUES('move', ?1, 'd', 'done', 0, 1, ?2)", params![src, batch]).unwrap();
        };
        for n in 0..300 {
            insert(format!("organized {n}"), Some("organize-1"));
        }
        for n in 0..250 {
            insert(format!("single {n}"), None);
        }
        let newest = list(&conn, 200).unwrap();
        assert_eq!(newest.len(), 200, "the 200 newest entries are single changes");
        assert!(newest.iter().all(|r| r.batch.is_none()));
        let all = list(&conn, 251).unwrap();
        assert_eq!(all.len(), 550, "the run comes back whole as the 251st entry");
        assert_eq!(all.iter().filter(|r| r.batch.as_deref() == Some("organize-1")).count(), 300);
        assert!(all.windows(2).all(|w| w[0].id > w[1].id), "newest first");
        insert("after".into(), Some("move-2"));
        insert("after too".into(), Some("move-2"));
        let top = list(&conn, 2).unwrap();
        assert_eq!(top.len(), 3, "a two-row batch and one single change");
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
        let error = rename_file(&conn, &roots, &src, "Renamed", false).unwrap_err();
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
        let error = record(&conn, OpKind::Move, &file, None, false).unwrap_err();
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
        assert_eq!(undo(&conn, &roots, rows[0].id).unwrap_err().code, ErrorCode::Validation, "a refused row has nothing to undo");
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
        undo(&conn, &roots, rows[0].id).unwrap();
        assert!(file.is_file(), "Undo brings it back");
        assert_eq!(get(&conn, rows[0].id).unwrap().unwrap().status, "undone");

        let rows = recycle(&conn, &roots, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rows[0].status, "done", "{:?}", rows[0].message);
        std::fs::write(&file, b"another file with that name").unwrap();
        undo(&conn, &roots, rows[0].id).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"another file with that name", "the file that took its place is never touched");
        let beside = roots.downloads.clone().unwrap().join("Tote recycle check (2).txt");
        assert_eq!(std::fs::read(&beside).unwrap(), b"recycle check", "the recycled file comes back beside it with its extension");
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
        undo(&conn, &roots, rows[0].id).unwrap();
        assert!(first.is_file(), "and Undo works on it");
    }

    /// Every file in `folder`, by name.
    fn names_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        names.sort();
        names
    }

    /// A file another program holds open with read sharing (the way Word and
    /// many PDF readers hold a document) used to be copied and then left
    /// behind, so she had two (audit T-13). On one drive the rename is
    /// refused and nothing is copied; between drives the copy is made, the
    /// original cannot be removed, and the copy is taken back out. Either
    /// way exactly one file stays and she reads that it is open elsewhere.
    #[cfg(windows)]
    #[test]
    fn a_file_held_open_with_read_sharing_leaves_exactly_one_file() {
        let (_dir, roots, conn) = fixture();
        let src = roots.documents.clone().unwrap().join("Grade 7 Quiz.docx");
        std::fs::write(&src, b"the only copy").unwrap();
        let dest = roots.desktop.clone().unwrap().join("Quizzes");
        std::fs::create_dir_all(&dest).unwrap();
        let _held = {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 0x1;
            std::fs::OpenOptions::new().read(true).write(true).share_mode(FILE_SHARE_READ).open(&src).unwrap()
        };
        let error = move_file(&conn, &roots, &src, &dest, OpKind::Move).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (ErrorCode::InUse, IN_USE));
        assert_eq!(list(&conn, 1).unwrap()[0].message.as_deref(), Some(IN_USE), "Activity carries the same sentence");
        assert!(names_in(&dest).is_empty(), "nothing was copied: {:?}", names_in(&dest));
        // The path between drives, where the copy itself succeeds.
        let error = copy_across(&src, &dest.join("Grade 7 Quiz.docx")).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (ErrorCode::InUse, IN_USE));
        assert!(names_in(&dest).is_empty(), "the copy was taken back out and no temporary copy stayed: {:?}", names_in(&dest));
        assert_eq!(std::fs::read(&src).unwrap(), b"the only copy", "the original is untouched");
    }

    /// A move between drives copies under a temporary name and gives the
    /// copy its real name only when it is whole; the original goes last,
    /// the web stamp travels with the copy where the drive can hold it, and
    /// a temporary name another copy holds is passed over, never written
    /// into (audit T-14, T-43).
    #[cfg(windows)]
    #[test]
    fn a_copy_between_drives_lands_whole_under_its_own_name() {
        let (_dir, roots, _conn) = fixture();
        let src = roots.downloads.clone().unwrap().join("DepEd Memo 123.pdf");
        std::fs::write(&src, b"%PDF-1.4 the memo").unwrap();
        std::fs::write(format!("{}:Zone.Identifier", src.display()), "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://www.deped.gov.ph/memo.pdf\r\n").unwrap();
        let dest = roots.desktop.clone().unwrap().join("Memos");
        std::fs::create_dir_all(&dest).unwrap();
        // Another copy to the same name is under way.
        let other = dest.join(format!("{PARTIAL_PREFIX}DepEd Memo 123.pdf"));
        std::fs::write(&other, b"another copy").unwrap();
        let dst = dest.join("DepEd Memo 123.pdf");
        copy_across(&src, &dst).unwrap();
        assert!(!src.exists(), "the original is removed once the copy is whole");
        assert_eq!(std::fs::read(&dst).unwrap(), b"%PDF-1.4 the memo");
        assert!(origin::read_zone(&dst).is_some(), "the web stamp came along on a drive that holds it");
        assert_eq!(std::fs::read(&other).unwrap(), b"another copy", "the other copy's temporary file was never written into");
        assert_eq!(names_in(&dest), vec![format!("{PARTIAL_PREFIX}DepEd Memo 123.pdf"), "DepEd Memo 123.pdf".to_string()], "no temporary copy of its own stayed");
    }

    /// A rename or a copy never replaces a file that is there, whoever put
    /// it there: the move's lock no longer keeps two moves apart while a
    /// file is copied, so the last step itself refuses (audit T-12).
    #[test]
    fn a_move_never_replaces_a_file_that_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.docx"), dir.path().join("b.docx"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert_eq!(place(&a, &b).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(copy_new(&a, &b).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()), "both files are untouched");
        let c = dir.path().join("c.docx");
        place(&a, &c).unwrap();
        assert!(!a.exists() && std::fs::read(&c).unwrap() == b"a", "a free name is taken");
    }

    /// Past 260 characters a direct Win32 call needs the long form of the
    /// path; a deep file still moves on one drive and between drives.
    #[cfg(windows)]
    #[test]
    fn a_file_with_a_long_path_still_moves() {
        let (_dir, roots, conn) = fixture();
        let deep = roots.documents.clone().unwrap().join("A".repeat(120)).join("B".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        let file = deep.join("Deep Quiz.docx");
        std::fs::write(&file, b"deep").unwrap();
        assert!(path_units(&file) >= 260);
        let deeper = deep.join("C".repeat(30));
        let moved = move_file(&conn, &roots, &file, &deeper, OpKind::Move).unwrap();
        assert_eq!(std::fs::read(&moved).unwrap(), b"deep");
        let copied = deep.join("Deep Quiz copy.docx");
        copy_across(&moved, &copied).unwrap();
        assert_eq!(std::fs::read(&copied).unwrap(), b"deep");
        assert!(!moved.exists());
    }

    /// A database that writes down, for every hold, which of two paths
    /// held the file when the hold began and when it ended.
    struct Watched {
        conn: std::sync::Mutex<Connection>,
        from: PathBuf,
        to: PathBuf,
        holds: std::sync::Mutex<Vec<[(bool, bool); 2]>>,
    }

    struct WatchedHold<'a> {
        conn: std::sync::MutexGuard<'a, Connection>,
        watched: &'a Watched,
        began: (bool, bool),
    }

    impl Watched {
        fn new(conn: Connection, from: &Path, to: &Path) -> Self {
            Self { conn: std::sync::Mutex::new(conn), from: from.to_path_buf(), to: to.to_path_buf(), holds: Default::default() }
        }
        fn where_now(&self) -> (bool, bool) {
            (self.from.exists(), self.to.exists())
        }
    }

    impl std::ops::Deref for WatchedHold<'_> {
        type Target = Connection;
        fn deref(&self) -> &Connection {
            &self.conn
        }
    }

    impl Drop for WatchedHold<'_> {
        fn drop(&mut self) {
            let ended = self.watched.where_now();
            self.watched.holds.lock().unwrap().push([self.began, ended]);
        }
    }

    impl Db for Watched {
        type Held<'a>
            = WatchedHold<'a>
        where
            Self: 'a;
        fn hold(&self) -> Result<WatchedHold<'_>> {
            let conn = self.conn.lock().unwrap();
            Ok(WatchedHold { conn, watched: self, began: self.where_now() })
        }
    }

    /// The lock rule of audit T-12: a move and its Undo hold the database
    /// to record the change and to finish it, and the file moves between
    /// two holds, never during one, so a search is never stuck behind a
    /// copy to a flash drive.
    #[test]
    fn a_move_and_its_undo_never_hold_the_lock_while_the_file_moves() {
        let (_dir, roots, conn) = fixture();
        let src = roots.desktop.clone().unwrap().join("Class Video.mp4");
        std::fs::write(&src, b"video").unwrap();
        let dest = roots.documents.clone().unwrap().join("Videos");
        let watched = Watched::new(conn, &src, &dest.join("Class Video.mp4"));
        move_file(&watched, &roots, &src, &dest, OpKind::Move).unwrap();
        let id = list(&watched.hold().unwrap(), 1).unwrap()[0].id;
        undo(&watched, &roots, id).unwrap();
        assert!(src.is_file(), "Undo put it back");
        let holds = watched.holds.lock().unwrap().clone();
        assert!(holds.len() >= 4, "the move and the Undo each took the lock before and after: {holds:?}");
        for [began, ended] in &holds {
            assert_eq!(began, ended, "the file moved while the lock was held: {holds:?}");
        }
        assert!(holds.iter().any(|[began, _]| *began == (true, false)) && holds.iter().any(|[began, _]| *began == (false, true)), "holds on both sides of each move: {holds:?}");
    }

    /// The start-up sweep (audit T-14, T-48): a move that finished on disk
    /// is done and its index row follows it; a move cut short failed with
    /// the plain note and its temporary copy is gone, the original
    /// untouched; a folder that was made is done; a copy an Undo left
    /// half made is removed and the row can be undone again; files that are
    /// not Tote's temporary copies are never touched, and Activity shows
    /// every row.
    #[test]
    fn pending_rows_are_settled_and_cut_copies_removed_at_start() {
        let (_dir, roots, conn) = fixture();
        let (desktop, forms) = (roots.desktop.clone().unwrap(), roots.documents.clone().unwrap().join("Forms"));
        std::fs::create_dir_all(&forms).unwrap();
        let pending = |kind: &str, src: &Path, dst: Option<&Path>| {
            conn.execute(
                "INSERT INTO ops(kind, src, dst, status, automatic, created_at) VALUES(?1, ?2, ?3, 'pending', 0, 1)",
                params![kind, src.to_string_lossy().into_owned(), dst.map(|d| d.to_string_lossy().into_owned())],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        // Closed after the file landed but before the row was finished.
        let (landed_from, landed) = (desktop.join("SF9 Template.docx"), forms.join("SF9 Template.docx"));
        std::fs::write(&landed_from, b"sf9").unwrap();
        index::refresh_path(&conn, &landed_from).unwrap();
        std::fs::rename(&landed_from, &landed).unwrap();
        let finished = pending("move", &landed_from, Some(&landed));
        // Closed in the middle of the copy.
        let (video, video_dst) = (desktop.join("Class Video.mp4"), forms.join("Class Video.mp4"));
        std::fs::write(&video, b"the whole video").unwrap();
        let cut = forms.join(format!("{PARTIAL_PREFIX}Class Video.mp4"));
        std::fs::write(&cut, b"the whole").unwrap();
        let interrupted = pending("move", &video, Some(&video_dst));
        // A folder that was made.
        let folder = forms.join("Grade 7");
        std::fs::create_dir(&folder).unwrap();
        let made = pending("newFolder", &folder, None);
        // An Undo of a done move, closed in the middle of its copy back.
        let (quiz_away, quiz_home) = (forms.join("Quiz.docx"), desktop.join("Quiz.docx"));
        std::fs::write(&quiz_away, b"quiz").unwrap();
        conn.execute(
            "INSERT INTO ops(kind, src, dst, status, automatic, created_at, undone_to) VALUES('move', ?1, ?2, 'done', 0, 1, ?1)",
            params![quiz_home.to_string_lossy().into_owned(), quiz_away.to_string_lossy().into_owned()],
        )
        .unwrap();
        let undoing = conn.last_insert_rowid();
        let cut_back = desktop.join(format!("{PARTIAL_PREFIX}Quiz.docx"));
        std::fs::write(&cut_back, b"qu").unwrap();
        // Her own files beside them.
        std::fs::write(forms.join(".hidden note.txt"), b"hers").unwrap();
        std::fs::write(desktop.join("tote-partial-notes.txt"), b"hers too").unwrap();

        assert_eq!(resolve_pending(&conn).unwrap(), 3);
        let row = |id: i64| get(&conn, id).unwrap().unwrap();
        assert_eq!(row(finished).status, "done");
        assert!(index::by_path(&conn, &landed.to_string_lossy()).unwrap().is_some(), "the index row followed the file");
        assert_eq!((row(interrupted).status.as_str(), row(interrupted).message.as_deref()), ("failed", Some(CLOSED_BEFORE_DONE)));
        assert!(!cut.exists(), "the copy cut short is removed");
        assert_eq!(std::fs::read(&video).unwrap(), b"the whole video", "the original is whole where it was");
        assert!(!video_dst.exists(), "and no second copy appeared");
        assert_eq!(row(made).status, "done");
        assert!(!cut_back.exists(), "the copy an Undo was making is removed");
        assert_eq!(row(undoing).status, "done", "the Undo did not happen, so the row can be undone again");
        assert_eq!(undone(&conn, undoing).1, None, "and it no longer names a landing");
        assert!(forms.join(".hidden note.txt").is_file() && desktop.join("tote-partial-notes.txt").is_file(), "her own files stay");
        let shown: Vec<i64> = list(&conn, 10).unwrap().iter().map(|r| r.id).collect();
        for id in [finished, interrupted, made] {
            assert!(shown.contains(&id), "Activity shows change {id}: {shown:?}");
        }
        assert_eq!(resolve_pending(&conn).unwrap(), 0, "nothing is left pending");
        undo(&conn, &roots, undoing).unwrap();
        assert_eq!(std::fs::read(&quiz_home).unwrap(), b"quiz");
    }

    /// The two moments between the steps of a copy between drives where a
    /// closed Tote leaves whole files: the copy had its real name but the
    /// original was not yet removed (a second copy of the same size, which
    /// is removed, so she never has two), and an Undo whose file was already
    /// back (recorded as undone, its index row back at the old path).
    #[test]
    fn a_whole_second_copy_is_removed_and_a_finished_undo_recorded_at_start() {
        let (_dir, roots, conn) = fixture();
        let (desktop, forms) = (roots.desktop.clone().unwrap(), roots.documents.clone().unwrap().join("Forms"));
        std::fs::create_dir_all(&forms).unwrap();
        let (video, copy) = (desktop.join("Class Video.mp4"), forms.join("Class Video.mp4"));
        std::fs::write(&video, b"the whole video").unwrap();
        std::fs::write(&copy, b"the whole video").unwrap();
        // Her own file of another size at a pending move's destination is not Tote's copy.
        let (memo, memo_dst) = (desktop.join("Memo.pdf"), forms.join("Memo.pdf"));
        std::fs::write(&memo, b"the memo").unwrap();
        std::fs::write(&memo_dst, b"another memo of hers").unwrap();
        for (src, dst) in [(&video, &copy), (&memo, &memo_dst)] {
            conn.execute(
                "INSERT INTO ops(kind, src, dst, status, automatic, created_at) VALUES('move', ?1, ?2, 'pending', 0, 1)",
                params![src.to_string_lossy().into_owned(), dst.to_string_lossy().into_owned()],
            )
            .unwrap();
        }
        // An Undo that put the file back just before Tote was closed.
        let (quiz_away, quiz_home) = (forms.join("Quiz.docx"), desktop.join("Quiz.docx"));
        std::fs::write(&quiz_away, b"quiz").unwrap();
        index::refresh_path(&conn, &quiz_away).unwrap();
        std::fs::rename(&quiz_away, &quiz_home).unwrap();
        conn.execute(
            "INSERT INTO ops(kind, src, dst, status, automatic, created_at, undone_to) VALUES('move', ?1, ?2, 'done', 0, 1, ?1)",
            params![quiz_home.to_string_lossy().into_owned(), quiz_away.to_string_lossy().into_owned()],
        )
        .unwrap();
        let undid = conn.last_insert_rowid();

        assert_eq!(resolve_pending(&conn).unwrap(), 3);
        assert_eq!(std::fs::read(&video).unwrap(), b"the whole video", "the original stays");
        assert!(!copy.exists(), "the second copy is removed");
        assert_eq!((std::fs::read(&memo).unwrap(), std::fs::read(&memo_dst).unwrap()), (b"the memo".to_vec(), b"another memo of hers".to_vec()), "a file of another size is never removed");
        let statuses: Vec<String> = list(&conn, 10).unwrap().iter().map(|r| r.status.clone()).collect();
        assert_eq!(statuses, vec!["undone", "failed", "failed"]);
        let (undone_at, undone_to) = undone(&conn, undid);
        assert!(undone_at.is_some() && undone_to == Some(quiz_home.to_string_lossy().into_owned()), "the finished Undo is recorded with its landing");
        assert!(index::by_path(&conn, &quiz_home.to_string_lossy()).unwrap().is_some(), "and the index row followed the file back");
        assert_eq!(resolve_pending(&conn).unwrap(), 0);
    }

    /// A moved file that lost its web stamp on the way says so in its row,
    /// which the move's toast reads (audit T-43); a move that kept it does
    /// not. A FAT32 drive cannot be made here, so the loss is recorded
    /// through the same step a move takes.
    #[test]
    fn a_lost_web_stamp_is_recorded_in_the_row() {
        let (_dir, roots, conn) = fixture();
        let src = roots.downloads.clone().unwrap().join("Memo.pdf");
        std::fs::write(&src, b"%PDF").unwrap();
        let moved = move_file(&conn, &roots, &src, &roots.desktop.clone().unwrap(), OpKind::Move).unwrap();
        let row = list(&conn, 1).unwrap().remove(0);
        assert!(!row.stamp_lost, "a file without a stamp has none to lose");
        note_stamp_lost(&conn, row.id, &moved).unwrap();
        assert!(get(&conn, row.id).unwrap().unwrap().stamp_lost);
        #[cfg(windows)]
        {
            let stamped = roots.downloads.clone().unwrap().join("Form.pdf");
            std::fs::write(&stamped, b"%PDF").unwrap();
            std::fs::write(format!("{}:Zone.Identifier", stamped.display()), "[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
            let landed = move_file(&conn, &roots, &stamped, &roots.desktop.clone().unwrap(), OpKind::Move).unwrap();
            assert!(origin::read_zone(&landed).is_some());
            assert!(!list(&conn, 1).unwrap()[0].stamp_lost, "the stamp stayed on this drive");
        }
    }
}
