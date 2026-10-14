//! Shared state: the database behind a mutex, the roots, and the index status
//! the scanner reports. Background threads hold an `Arc<Core>`.

use crate::error::Result;
use crate::index::IndexStatus;
use crate::paths::Roots;
use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard};

pub struct Core {
    pub db: Mutex<Connection>,
    pub roots: Roots,
    pub status: Mutex<IndexStatus>,
    pub recovered: bool,
    /// Where tote.sqlite and tasks.sqlite live (the daily snapshot goes here too).
    pub data_dir: std::path::PathBuf,
    /// Set when a damaged tasks.sqlite was replaced on this start.
    pub tasks_restored: Option<crate::tasks::Restored>,
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
