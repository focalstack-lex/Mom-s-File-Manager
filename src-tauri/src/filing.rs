//! Move to Tote (Lex's decision, 2026-10-17): nothing is filed on its own;
//! she right-clicks a file in File Explorer and picks Move to Tote. The
//! installer adds that entry for her account (`windows/hooks.nsh`), and
//! Explorer starts `tote.exe --move-to-tote "<file>"` once per selected file.
//! A second Tote hands its arguments to the running one through the
//! single-instance plugin; the first one reads its own (lib.rs). Files that
//! arrive within `GATHER` of each other are filed together, by the Organize
//! rule (`organize::destination_for`), as one batch: one notification, one
//! Activity row with Undo all.
//!
//! Files dragged onto Tote's window join the same queue (lib.rs, the
//! window's drop event), so a drop files them exactly like Move to Tote.

use crate::error::{ErrorCode, ToteError};
use crate::index;
use crate::ops;
use crate::organize::{self, SubjectRule};
use crate::paths::{self, Roots};
use crate::sorter;
use crate::state::{Core, Db};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// The argument the File Explorer entry passes before the file.
pub const ARG: &str = "--move-to-tote";
/// What the renderer hears after a Move to Tote filed something.
pub const SORTED_EVENT: &str = "tote://sorted";
/// Explorer starts one Tote per selected file, a few milliseconds apart; a
/// file that arrives within this of the last one joins its batch.
const GATHER: Duration = Duration::from_millis(800);

/// The files a command line asks Tote to file: every argument after `ARG`.
/// Explorer passes a full path; anything else is ignored.
pub fn requested(args: &[String]) -> Vec<PathBuf> {
    let Some(at) = args.iter().position(|a| a == ARG) else { return Vec::new() };
    args[at + 1..].iter().map(PathBuf::from).filter(|p| p.is_absolute()).collect()
}

/// One file that went where it belongs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Landed {
    pub name: String,
    /// "School Files > Grade 7 > Science".
    pub folder: String,
    pub path: String,
}

/// One file left where it was, with the sentence that says why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Filed {
    pub moved: Vec<Landed>,
    pub failed: Vec<Refused>,
}

/// What the renderer hears: the first file, its folder, and how many went.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sorted {
    pub name: String,
    pub folder: String,
    pub path: String,
    pub count: usize,
}

/// "School Files > Grade 7 > Science" for a folder under School Files.
fn folder_label(school: &Path, dir: &Path) -> String {
    let mut label = String::from("School Files");
    if let Ok(rest) = dir.strip_prefix(school) {
        for part in rest.components() {
            label.push_str(" > ");
            label.push_str(&part.as_os_str().to_string_lossy());
        }
    }
    label
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| paths::display(path))
}

/// Why Tote leaves a file where it is before trying to move it: a folder
/// (dropped on the window) is hers to arrange, a shortcut or a system file
/// belongs where it is, and a download the browser is still writing (a
/// `.part` or `.crdownload` beside it) is not a file yet.
fn not_ready(path: &Path) -> Option<ToteError> {
    if path.is_dir() {
        return Some(ToteError::validation("Folders stay where they are. Open the folder and drag the files inside instead."));
    }
    let ext = sorter::extension(path);
    let name = name_of(path);
    if matches!(ext.as_str(), "lnk" | "url" | "ini" | "pif") || name.eq_ignore_ascii_case("desktop.ini") {
        return Some(ToteError::validation("Shortcuts and system files stay where they are. Pick the file itself instead."));
    }
    if crate::stability::has_partial_sibling(path) {
        return Some(ToteError::new(ErrorCode::InUse, "It is still downloading. Try again when the download is done."));
    }
    None
}

/// Files each path where the Organize rule says it belongs. Several files
/// are one batch (one Activity row, one Undo all); a file that cannot move
/// stays, is named in `failed`, and the others still go. A file already in
/// its folder, or in a folder she made inside it, is left there.
pub fn file_all(db: &impl Db, roots: &Roots, rules: &[SubjectRule], wanted: &[PathBuf]) -> Filed {
    let mut filed = Filed::default();
    let school = match roots.school_files() {
        Ok(school) => school,
        Err(error) => {
            filed.failed = wanted.iter().map(|p| Refused { name: name_of(p), message: error.message.clone() }).collect();
            return filed;
        }
    };
    let mut unique: Vec<&PathBuf> = Vec::new();
    for path in wanted {
        if !unique.iter().any(|seen| paths::same_path(seen, path)) {
            unique.push(path);
        }
    }
    let batch = (unique.len() > 1).then(|| ops::new_batch("tote"));
    for path in unique {
        let name = name_of(path);
        let result = (|| -> crate::error::Result<Landed> {
            if let Some(refusal) = not_ready(path) {
                return Err(refusal);
            }
            let modified = index::entry_for(path).map(|e| e.modified).unwrap_or_else(crate::db::now);
            let (dest, _why) = organize::destination_for(&school, &name, modified, rules);
            let resolved = paths::resolve(path)?;
            if resolved.parent().is_some_and(|parent| parent.starts_with(&dest)) {
                return Ok(Landed { name: name.clone(), folder: folder_label(&school, resolved.parent().unwrap_or(&dest)), path: paths::display(&resolved) });
            }
            let landed = match &batch {
                Some(batch) => ops::file_in_batch(db, roots, path, &dest, batch)?,
                None => ops::move_file(db, roots, path, &dest, ops::OpKind::AutoSort)?,
            };
            Ok(Landed { name: name_of(&landed), folder: folder_label(&school, &dest), path: paths::display(&landed) })
        })();
        match result {
            Ok(landed) => {
                // The kind, not the name: names often carry a student's name (audit T-52).
                log::info!("Move to Tote: {} is in {}", sorter::describe(path), landed.folder);
                filed.moved.push(landed);
            }
            Err(error) => {
                log::warn!("Move to Tote: {} stays ({})", sorter::describe(path), error.message);
                log::debug!("Move to Tote left {}", path.display());
                filed.failed.push(Refused { name, message: error.message });
            }
        }
    }
    filed
}

/// The notification for one Move to Tote: what went where, or why not.
pub fn summary(filed: &Filed) -> (String, String) {
    let (moved, failed) = (filed.moved.len(), filed.failed.len());
    match (filed.moved.first(), filed.failed.first()) {
        (Some(only), None) if moved == 1 => ("Saved for you".to_string(), format!("{} is in {}.", only.name, only.folder)),
        (Some(first), None) => {
            let same = filed.moved.iter().all(|m| m.folder == first.folder);
            let folder = if same { first.folder.as_str() } else { "School Files" };
            ("Saved for you".to_string(), format!("{moved} files are in {folder}."))
        }
        (Some(_), Some(refused)) => (format!("Saved {moved} of {} files", moved + failed), format!("{}: {}", refused.name, refused.message)),
        (None, Some(refused)) if failed == 1 => (format!("Tote could not file {}", refused.name), refused.message.clone()),
        (None, Some(refused)) => ("Tote could not file those files".to_string(), format!("{}: {}", refused.name, refused.message)),
        (None, None) => (String::new(), String::new()),
    }
}

/// Gathers the files sent to Tote and files them on a thread of its own,
/// so the single-instance callback (on the main thread) only queues them.
/// The receiver ends when every sender is gone.
pub fn start(core: Arc<Core>, app: AppHandle, letters: mpsc::Receiver<PathBuf>) {
    let spawned = std::thread::Builder::new().name("tote-filing".into()).spawn(move || {
        while let Ok(first) = letters.recv() {
            let mut wanted = vec![first];
            while let Ok(more) = letters.recv_timeout(GATHER) {
                wanted.push(more);
            }
            // Settings before any lock: the database mutex is not reentrant.
            let rules = organize::effective_rules(&crate::commands::settings_of(&core).subject_rules);
            let filed = file_all(&*core, &core.roots, &rules, &wanted);
            tell(&app, &filed);
        }
    });
    if let Err(error) = spawned {
        log::error!("could not start the Move to Tote thread: {error}");
    }
}

/// A notification answers her click every time (it is not a background
/// event, so the Settings switch for those does not apply), and the window,
/// when open, hears the files that went.
fn tell(app: &AppHandle, filed: &Filed) {
    let (title, body) = summary(filed);
    if !title.is_empty() {
        use tauri_plugin_notification::NotificationExt;
        if let Err(error) = app.notification().builder().title(&title).body(&body).show() {
            log::warn!("Move to Tote notification: {error}");
        }
    }
    if let Some(first) = filed.moved.first() {
        let same = filed.moved.iter().all(|m| m.folder == first.folder);
        let folder = if same { first.folder.clone() } else { "School Files".to_string() };
        let _ = app.emit(SORTED_EVENT, Sorted { name: first.name.clone(), folder, path: first.path.clone(), count: filed.moved.len() });
    }
    let _ = app.emit(crate::worker::CHANGED_EVENT, ());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::db;

    fn fixture() -> (tempfile::TempDir, Roots, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let base = paths::resolve(dir.path()).unwrap();
        let (desktop, documents, downloads) = (base.join("Desktop"), base.join("Documents"), base.join("Downloads"));
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        (dir, Roots::explicit(desktop, documents, downloads), db::open_memory().unwrap())
    }

    #[test]
    fn the_files_after_the_flag_are_the_ones_asked_for() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let abs = if cfg!(windows) { r"C:\Users\Her\Downloads\Quiz.docx" } else { "/home/her/Downloads/Quiz.docx" };
        assert_eq!(requested(&args(&["tote.exe", ARG, abs])), vec![PathBuf::from(abs)]);
        assert!(requested(&args(&["tote.exe", "--minimized"])).is_empty(), "a plain start asks for nothing");
        assert!(requested(&args(&["tote.exe", ARG, "Quiz.docx"])).is_empty(), "a path that is not complete is ignored");
        assert!(requested(&args(&["tote.exe", ARG])).is_empty());
    }

    /// One file goes where Organize would put it, by grade and subject, and
    /// Activity can undo it; one in a batch of several shares one Undo all.
    #[test]
    fn a_file_goes_where_organize_would_put_it() {
        let (_dir, roots, conn) = fixture();
        let downloads = roots.downloads.clone().unwrap();
        let school = roots.school_files().unwrap();
        let quiz = downloads.join("Grade 7 Science Quiz.docx");
        std::fs::write(&quiz, b"q").unwrap();
        let filed = file_all(&conn, &roots, &organize::default_rules(), std::slice::from_ref(&quiz));
        assert!(filed.failed.is_empty(), "{:?}", filed.failed);
        let landed = school.join("Grade 7").join("Science").join("Grade 7 Science Quiz.docx");
        assert!(landed.is_file() && !quiz.exists());
        assert_eq!(filed.moved[0].folder, "School Files > Grade 7 > Science");
        assert_eq!(summary(&filed), ("Saved for you".to_string(), "Grade 7 Science Quiz.docx is in School Files > Grade 7 > Science.".to_string()));
        let rows = ops::list(&conn, 10).unwrap();
        assert_eq!((rows[0].kind.as_str(), rows[0].status.as_str(), rows[0].batch.as_deref()), ("autoSort", "done", None));
        ops::undo(&conn, &roots, rows[0].id).unwrap();
        assert!(quiz.is_file(), "Undo in Activity puts it back");
    }

    #[test]
    fn several_files_are_one_batch_and_a_refused_one_stays() {
        let (_dir, roots, conn) = fixture();
        let desktop = roots.desktop.clone().unwrap();
        let memo = desktop.join("DepEd Memo.pdf");
        let quiz = desktop.join("Grade 8 Math Quiz.xlsx");
        let link = desktop.join("Zoom.lnk");
        let empty = desktop.join("Still downloading.pdf");
        for (file, body) in [(&memo, &b"%PDF"[..]), (&quiz, b"x"), (&link, b"l"), (&empty, b"")] {
            std::fs::write(file, body).unwrap();
        }
        // Firefox keeps the empty file beside the part it is still writing.
        std::fs::write(desktop.join("Still downloading.pdf.part"), b"half").unwrap();
        let filed = file_all(&conn, &roots, &organize::default_rules(), &[memo.clone(), quiz.clone(), link.clone(), empty.clone(), memo.clone()]);
        assert_eq!(filed.moved.len(), 2, "the same file asked twice moves once");
        assert_eq!(filed.failed.len(), 2);
        assert!(link.is_file() && empty.is_file(), "the shortcut and the file still downloading stay");
        assert_eq!(filed.failed[1].message, "It is still downloading. Try again when the download is done.");
        let rows = ops::list(&conn, 10).unwrap();
        let batch = rows[0].batch.clone().unwrap();
        assert!(rows.iter().filter(|r| r.status == "done").all(|r| r.batch.as_deref() == Some(batch.as_str())), "one Activity entry");
        let (title, body) = summary(&filed);
        assert_eq!(title, "Saved 2 of 4 files");
        assert!(body.starts_with("Zoom.lnk: "));
        let undone = ops::undo_batch(&conn, &roots, &batch).unwrap();
        assert_eq!(undone.undone, 2);
        assert!(memo.is_file() && quiz.is_file());
    }

    /// A file outside her folders is refused with the boundary's own
    /// sentence, and one already in its folder (or a folder she made inside
    /// it) is left where it is, with no Activity row.
    #[test]
    fn outside_her_folders_is_refused_and_already_filed_stays() {
        let (dir, roots, conn) = fixture();
        let elsewhere = dir.path().join("Flash drive");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let outside = elsewhere.join("Grade 7 Quiz.docx");
        std::fs::write(&outside, b"q").unwrap();
        let filed = file_all(&conn, &roots, &organize::default_rules(), std::slice::from_ref(&outside));
        assert_eq!(filed.failed.len(), 1);
        assert!(outside.is_file());
        assert_eq!(summary(&filed).0, "Tote could not file Grade 7 Quiz.docx");

        let week = roots.school_files().unwrap().join("Grade 7").join("Week 3");
        std::fs::create_dir_all(&week).unwrap();
        let kept = week.join("Grade 7 Quiz.docx");
        std::fs::write(&kept, b"q").unwrap();
        let filed = file_all(&conn, &roots, &organize::default_rules(), std::slice::from_ref(&kept));
        assert!(filed.failed.is_empty() && kept.is_file(), "her own folder inside the rule's folder is kept");
        assert_eq!(filed.moved[0].folder, "School Files > Grade 7 > Week 3");
        assert!(ops::list(&conn, 10).unwrap().is_empty(), "nothing moved, nothing to undo");
    }

    /// A folder dropped on the window is refused with a sentence about
    /// folders, not "the file is gone", and the files beside it still go.
    #[test]
    fn a_dropped_folder_stays_with_a_sentence_about_folders() {
        let (_dir, roots, conn) = fixture();
        let downloads = roots.downloads.clone().unwrap();
        let folder = downloads.join("Grade 7 Science");
        std::fs::create_dir_all(&folder).unwrap();
        let quiz = downloads.join("Grade 7 Science Quiz.docx");
        std::fs::write(&quiz, b"q").unwrap();
        let filed = file_all(&conn, &roots, &organize::default_rules(), &[folder.clone(), quiz.clone()]);
        assert_eq!(filed.moved.len(), 1, "{:?}", filed.failed);
        assert_eq!(filed.failed.len(), 1);
        assert!(filed.failed[0].message.starts_with("Folders stay where they are"), "{}", filed.failed[0].message);
        assert!(folder.is_dir() && !quiz.exists());
    }
}
