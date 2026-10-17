//! Shared state: the database behind a mutex, the roots, and the index status
//! the scanner reports. Background threads hold an `Arc<Core>`.

use crate::error::Result;
use crate::index::IndexStatus;
use crate::paths::Roots;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long a path Tote placed in Downloads is skipped by the watcher: long
/// enough for every event a slow copy raises, short enough that a later file
/// at the same path is a download again.
pub const PLACED_FOR: Duration = Duration::from_secs(10 * 60);

/// Paths Tote itself just put in Downloads (Undo, restore, Undo all), each
/// with the moment, so the Downloads watcher does not file them again
/// (audit T-06). Paths compare without case, the way Windows does.
#[derive(Default)]
pub struct Placed(Mutex<HashMap<String, Instant>>);

impl Placed {
    fn key(path: &Path) -> String {
        path.to_string_lossy().trim_end_matches('\\').to_lowercase()
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records that Tote is about to put a file at `path`.
    pub fn note(&self, path: &Path) {
        self.note_at(path, Instant::now());
    }

    pub fn note_at(&self, path: &Path, now: Instant) {
        self.entries().insert(Self::key(path), now);
    }

    /// True while `path` was placed less than `PLACED_FOR` ago; entries past
    /// that are forgotten here, so the map never grows.
    pub fn covers(&self, path: &Path, now: Instant) -> bool {
        let mut entries = self.entries();
        entries.retain(|_, at| now.duration_since(*at) < PLACED_FOR);
        entries.contains_key(&Self::key(path))
    }
}

pub struct Core {
    pub db: Mutex<Connection>,
    pub roots: Roots,
    pub status: Mutex<IndexStatus>,
    pub recovered: bool,
    /// Set when tote.sqlite was rebuilt on this start; `kept` says whether
    /// her settings, pins, templates and Activity came along.
    pub rebuilt: Option<crate::db::Rebuilt>,
    /// Where tote.sqlite and tasks.sqlite live (the daily snapshot goes here too).
    pub data_dir: std::path::PathBuf,
    /// Set when a damaged tasks.sqlite was replaced on this start.
    pub tasks_restored: Option<crate::tasks::Restored>,
    /// The To do store could not be opened on this start; its commands fail.
    pub tasks_unavailable: bool,
    /// What Tote itself just put in Downloads, for the watcher to skip.
    pub placed: Placed,
}

impl Core {
    /// The database lock. A thread that panicked while holding it leaves it
    /// poisoned; SQLite keeps its own consistency (each statement is atomic),
    /// so the guard is recovered instead of failing every later command.
    pub fn db(&self) -> Result<MutexGuard<'_, Connection>> {
        Ok(self.db.lock().unwrap_or_else(|poisoned| {
            log::warn!("database lock recovered after a panic in another thread");
            poisoned.into_inner()
        }))
    }

    pub fn status(&self) -> IndexStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_status(&self, update: impl FnOnce(&mut IndexStatus)) {
        if let Ok(mut status) = self.status.lock() {
            update(&mut status);
        }
    }
}

pub struct AppState(pub Arc<Core>);

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A path Tote placed is skipped, in any spelling of its case, until its
    /// time is up; a path never placed is not.
    #[test]
    fn a_placed_path_is_covered_until_its_time_is_up() {
        let placed = Placed::default();
        let start = Instant::now();
        let back = PathBuf::from(r"C:\Users\Her\Downloads\Quiz A.docx");
        assert!(!placed.covers(&back, start));
        placed.note_at(&back, start);
        assert!(placed.covers(&back, start + Duration::from_secs(2)));
        assert!(placed.covers(Path::new(r"c:\users\her\downloads\quiz a.docx"), start + Duration::from_secs(30)), "the watcher may spell the path in another case");
        assert!(placed.covers(&back, start + PLACED_FOR - Duration::from_secs(1)), "every event within the window is skipped, not only the first");
        assert!(!placed.covers(&back, start + PLACED_FOR), "forgotten after its time");
        assert!(!placed.covers(&back, start + PLACED_FOR + Duration::from_secs(1)));
        assert!(!placed.covers(Path::new(r"C:\Users\Her\Downloads\Quiz B.docx"), start));
    }
}
