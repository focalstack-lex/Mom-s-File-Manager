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
    /// Set when tote.sqlite was rebuilt on this start; `kept` says whether
    /// her settings, pins, templates and Activity came along.
    pub rebuilt: Option<crate::db::Rebuilt>,
    /// Where tote.sqlite and tasks.sqlite live (the daily snapshot goes here too).
    pub data_dir: std::path::PathBuf,
    /// Set when a damaged tasks.sqlite was replaced on this start.
    pub tasks_restored: Option<crate::tasks::Restored>,
    /// The To do store could not be opened on this start; its commands fail.
    pub tasks_unavailable: bool,
}

/// Where work that gives the database lock back between steps takes it
/// from: the shared lock in the app (`Core`), or a connection the caller
/// already holds (a guard it took, or a plain connection in tests), which
/// is then used as it is with no second lock. A move holds it to record the
/// change and to finish it, never while the file is copied (audit T-12).
pub trait Db {
    type Held<'a>: std::ops::Deref<Target = Connection>
    where
        Self: 'a;
    fn hold(&self) -> Result<Self::Held<'_>>;
}

impl Db for Core {
    type Held<'a>
        = MutexGuard<'a, Connection>
    where
        Self: 'a;
    fn hold(&self) -> Result<MutexGuard<'_, Connection>> {
        self.db()
    }
}

impl Db for Connection {
    type Held<'a>
        = &'a Connection
    where
        Self: 'a;
    fn hold(&self) -> Result<&Connection> {
        Ok(self)
    }
}

impl Db for MutexGuard<'_, Connection> {
    type Held<'a>
        = &'a Connection
    where
        Self: 'a;
    fn hold(&self) -> Result<&Connection> {
        Ok(self)
    }
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
