//! The one error type every command returns. Serialized for the renderer as
//! `{ code, message, path? }`. `message` is always one plain sentence with a
//! next step, the sentence she reads; raw OS or library text goes to the log
//! only (audit T-17, 2026-10-16). The renderer falls back to one fixed
//! sentence per `code` when `message` is missing.

use serde::Serialize;

/// Stable error categories the renderer can map to plain sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    InvalidPath,
    InvalidName,
    OutsideRoots,
    Io,
    Db,
    Validation,
    Unsupported,
    /// The update check or download could not reach codewithlex.site.
    Network,
    /// The file is open in another program (a Windows sharing violation).
    InUse,
    /// Undo of a Recycle Bin change found nothing to restore.
    NotInRecycleBin,
    /// The file's drive has no Recycle Bin, or its path is too long for it.
    NoRecycleBin,
    /// The downloaded update did not match Tote's key.
    UpdateSignature,
    /// The installer could not run.
    UpdateInstall,
}

/// What a failed database read says; `From<rusqlite::Error>` assumes a read.
pub const DB_READ: &str = "Tote could not read its list right now. Try again in a moment.";
/// What a failed database write says.
pub const DB_WRITE: &str = "Tote could not save that. Try again in a moment.";
/// What a file another program holds open says (a Windows sharing violation).
pub const IN_USE: &str = "That file is open in another program. Close it there, then try again.";
/// What a file that is no longer at its path says, wherever that is found.
pub const FILE_GONE: &str = "That file is no longer there. Search for it in Find.";
/// What a name already taken in the folder says, wherever that is found.
pub const NAME_TAKEN: &str = "A file with that name already exists there. Pick another name.";

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct ToteError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ToteError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), path: None }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Validation, message)
    }

    /// A `Db` error with a message of the caller's own, for failures that
    /// never reach her (db.rs reports them at start and in the log).
    pub fn db(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Db, message)
    }

    /// A database read that failed: the raw error is logged, she reads `DB_READ`.
    pub fn db_read(error: impl std::fmt::Display) -> Self {
        log::warn!("database read: {error}");
        Self::new(ErrorCode::Db, DB_READ)
    }

    /// A database write that failed: the raw error is logged, she reads `DB_WRITE`.
    pub fn db_write(error: impl std::fmt::Display) -> Self {
        log::warn!("database write: {error}");
        Self::new(ErrorCode::Db, DB_WRITE)
    }

    /// Rewords a `Db` error that came through `From` (which assumes a read)
    /// for a caller that was writing, such as through `db::set_setting`. Any
    /// other code passes through unchanged.
    pub fn as_write(mut self) -> Self {
        if self.code == ErrorCode::Db {
            self.message = DB_WRITE.into();
        }
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// The code and sentence for an io error. Windows reports a file another
/// program holds open as a sharing or lock violation (os errors 32 and 33),
/// which std leaves uncategorized, so the raw code is read first.
pub fn io_words(error: &std::io::Error) -> (ErrorCode, &'static str) {
    use std::io::ErrorKind;
    if cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)) {
        return (ErrorCode::InUse, IN_USE);
    }
    match error.kind() {
        ErrorKind::NotFound => (ErrorCode::NotFound, FILE_GONE),
        ErrorKind::PermissionDenied => (ErrorCode::PermissionDenied, "Windows did not allow that change. Check the file in File Explorer, then try again."),
        ErrorKind::AlreadyExists => (ErrorCode::AlreadyExists, NAME_TAKEN),
        ErrorKind::StorageFull => (ErrorCode::Io, "The drive is full. Free some space, then try again."),
        _ => (ErrorCode::Io, "Tote could not finish that change. Try again in a moment."),
    }
}

impl From<std::io::Error> for ToteError {
    fn from(error: std::io::Error) -> Self {
        let (code, sentence) = io_words(&error);
        log::warn!("io: {error}");
        Self::new(code, sentence)
    }
}

impl From<rusqlite::Error> for ToteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::db_read(error)
    }
}

pub type Result<T> = std::result::Result<T, ToteError>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_their_category() {
        let error: ToteError = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert_eq!(error.code, ErrorCode::NotFound);
        let error: ToteError = std::io::Error::other("other").into();
        assert_eq!(error.code, ErrorCode::Io);
        let error: ToteError = std::io::Error::new(std::io::ErrorKind::StorageFull, "full").into();
        assert_eq!(error.code, ErrorCode::Io);
        assert!(error.message.starts_with("The drive is full."));
    }

    /// Windows reports a file held open by another program as os error 32
    /// (sharing) or 33 (lock); std leaves both uncategorized.
    #[cfg(windows)]
    #[test]
    fn a_sharing_violation_means_the_file_is_open_elsewhere() {
        for code in [32, 33] {
            let error: ToteError = std::io::Error::from_raw_os_error(code).into();
            assert_eq!(error.code, ErrorCode::InUse, "os error {code}");
            assert_eq!(error.message, IN_USE);
        }
    }

    /// The sentence she reads never carries the OS text (audit T-17): every
    /// io error, whatever Windows said, becomes one plain sentence that ends
    /// with what to do next.
    #[test]
    fn no_io_error_puts_os_text_in_the_message() {
        use std::io::ErrorKind;
        let mut errors: Vec<std::io::Error> = [2, 3, 5, 32, 33, 112, 1155, 1314].into_iter().map(std::io::Error::from_raw_os_error).collect();
        for kind in [ErrorKind::NotFound, ErrorKind::PermissionDenied, ErrorKind::AlreadyExists, ErrorKind::StorageFull, ErrorKind::Interrupted, ErrorKind::TimedOut, ErrorKind::Other] {
            errors.push(std::io::Error::new(kind, "The process cannot access the file because it is being used by another process. (os error 32)"));
        }
        errors.push(std::io::Error::other("Error during a `trash` operation: Unknown { description: \"Some operations were aborted\" }"));
        for error in errors {
            let raw = error.to_string();
            let tote: ToteError = error.into();
            for technical in ["os error", "Error during", "process", "{", "`"] {
                assert!(!tote.message.contains(technical), "{raw:?} became {:?}, which carries {technical:?}", tote.message);
            }
            assert!(tote.message.ends_with('.') && tote.message.contains(". "), "one sentence plus a next step: {:?}", tote.message);
        }
    }

    /// The string literal passed as the message of each `ToteError::new(`
    /// and `ToteError::validation(` call in `source`; a call whose message
    /// is a constant, a variable or a `format!` is skipped (those sentences
    /// are checked where they are defined).
    fn literal_sentences(source: &str) -> Vec<String> {
        // Each call shape, and whether a code argument comes before the message.
        let calls = [("ToteError::new(", true), ("ToteError::validation(", false)];
        let mut found = Vec::new();
        for (call, has_code) in calls {
            found.extend(source.match_indices(call).filter_map(|(index, _)| {
                let mut rest = &source[index + call.len()..];
                if has_code {
                    rest = &rest[rest.find(',')? + 1..];
                }
                let literal = rest.trim_start().strip_prefix('"')?;
                Some(literal[..literal.find('"')?].to_string())
            }));
        }
        found
    }

    /// Sentences that are the next step themselves: an instruction she reads
    /// under the field or the picker it is about, so nothing needs to follow.
    const OWN_NEXT_STEP: [&str; 4] = [
        "Type a name first.",
        "Type what needs doing first.",
        "Add the quarter dates in Settings first.",
        "Pick a folder inside your files, not a whole drive.",
    ];

    /// The renderer keeps a plain native sentence as it is, so a sentence
    /// without a next step would reach her as is (audit T-18 review). Every
    /// handwritten sentence in the modules that touch her files, her folders,
    /// her tasks and the update follows the rule the io table follows: one
    /// sentence, then what to do next.
    #[test]
    fn every_handwritten_sentence_carries_a_next_step() {
        let sources = [
            ("ops.rs", include_str!("ops.rs")),
            ("commands.rs", include_str!("commands.rs")),
            ("clipboard.rs", include_str!("clipboard.rs")),
            ("tasks.rs", include_str!("tasks.rs")),
            ("templates.rs", include_str!("templates.rs")),
            ("paths.rs", include_str!("paths.rs")),
            ("update.rs", include_str!("update.rs")),
            ("worker.rs", include_str!("worker.rs")),
            ("filing.rs", include_str!("filing.rs")),
            ("lib.rs", include_str!("lib.rs")),
        ];
        let mut sentences: Vec<(String, String)> = [DB_READ, DB_WRITE, IN_USE, FILE_GONE, NAME_TAKEN].iter().map(|s| ("error.rs".to_string(), s.to_string())).collect();
        for (file, source) in sources {
            // A file whose every sentence is a shared constant has no literal to scan.
            sentences.extend(literal_sentences(source).into_iter().map(|s| (file.to_string(), s)));
        }
        assert!(sentences.len() >= 40, "the scan found only {} sentences; the pattern may have drifted", sentences.len());
        for own in OWN_NEXT_STEP {
            assert!(sentences.iter().any(|(_, s)| s == own), "{own:?} is listed as its own next step but the scan no longer finds it");
        }
        for (file, sentence) in sentences {
            let instruction = OWN_NEXT_STEP.contains(&sentence.as_str());
            assert!(sentence.ends_with('.') && (instruction || sentence.contains(". ")), "{file}: {sentence:?} has no next step");
            for technical in ["os error", "Error during", "{", "`", "::"] {
                assert!(!sentence.contains(technical), "{file}: {sentence:?} carries {technical:?}");
            }
        }
    }

    #[test]
    fn database_errors_say_read_or_write() {
        let read: ToteError = rusqlite::Error::QueryReturnedNoRows.into();
        assert_eq!(read.code, ErrorCode::Db);
        assert_eq!(read.message, DB_READ);
        assert_eq!(read.as_write().message, DB_WRITE);
        let write = ToteError::db_write(rusqlite::Error::QueryReturnedNoRows);
        assert_eq!((write.code, write.message.as_str()), (ErrorCode::Db, DB_WRITE));
        assert_eq!(ToteError::validation("Type a name first.").as_write().message, "Type a name first.", "only a Db error is reworded");
    }

    #[test]
    fn serializes_in_camel_case_without_an_empty_path() {
        let json = serde_json::to_value(ToteError::validation("bad")).unwrap();
        assert_eq!(json, serde_json::json!({ "code": "validation", "message": "bad" }));
    }

    #[test]
    fn the_newer_codes_serialize_as_the_renderer_expects() {
        let codes = [
            (ErrorCode::InUse, "inUse"),
            (ErrorCode::NotInRecycleBin, "notInRecycleBin"),
            (ErrorCode::NoRecycleBin, "noRecycleBin"),
            (ErrorCode::UpdateSignature, "updateSignature"),
            (ErrorCode::UpdateInstall, "updateInstall"),
        ];
        for (code, text) in codes {
            assert_eq!(serde_json::to_value(code).unwrap(), serde_json::json!(text));
        }
    }
}
