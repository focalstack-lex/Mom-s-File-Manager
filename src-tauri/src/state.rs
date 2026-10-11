//! Shared state: the database behind a mutex, the roots, and the index status
//! the scanner reports. Background threads hold an `Arc<Core>`.

use crate::error::Result;
use crate::index::{self, IndexStatus};
use crate::paths::Roots;
use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard};

pub struct Core {
    pub db: Mutex<Connection>,
    pub roots: Roots,
    pub status: Mutex<IndexStatus>,
    pub recovered: bool,
}

impl Core {
    pub fn db(&self) -> Result<MutexGuard<'_, Connection>> {
        self.db.lock().map_err(|_| index::lock_err())
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
