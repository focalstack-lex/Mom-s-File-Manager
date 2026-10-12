//! The one SQLite database: the file index with full-text search, pins,
//! templates, the operations log and settings. A damaged database is moved
//! aside and rebuilt from disk, since her files are the real source of truth.

use crate::error::{DeskError, Result};
use rusqlite::Connection;
use std::path::Path;

pub struct Opened {
    pub connection: Connection,
    pub recovered: bool,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    parent TEXT NOT NULL,
    name TEXT NOT NULL,
    ext TEXT NOT NULL,
    kind TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    origin_label TEXT,
    origin_host TEXT,
    downloaded_at INTEGER,
    group_key TEXT NOT NULL,
    content_indexed INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS files_modified ON files(modified DESC);
CREATE INDEX IF NOT EXISTS files_parent ON files(parent);
CREATE INDEX IF NOT EXISTS files_group ON files(group_key);
CREATE INDEX IF NOT EXISTS files_downloaded ON files(downloaded_at DESC);
CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(name, content, tokenize = 'unicode61 remove_diacritics 2');
CREATE TABLE IF NOT EXISTS ops (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    src TEXT NOT NULL,
    dst TEXT,
    status TEXT NOT NULL,
    message TEXT,
    automatic INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pins (
    path TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS templates (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// Opens (or creates) `desk.sqlite` in `data_dir`.
pub fn open(data_dir: &Path) -> Result<Opened> {
    std::fs::create_dir_all(data_dir)?;
    let file = data_dir.join("desk.sqlite");
    match open_at(&file) {
        Ok(connection) => Ok(Opened { connection, recovered: false }),
        Err(error) => {
            log::warn!("database at {} is unusable ({error}); rebuilding", file.display());
            let aside = data_dir.join(format!("desk.damaged-{}.sqlite", chrono::Utc::now().timestamp()));
            if file.exists() {
                std::fs::rename(&file, &aside)?;
            }
            let connection = open_at(&file)?;
            Ok(Opened { connection, recovered: true })
        }
    }
}

fn open_at(file: &Path) -> Result<Connection> {
    let connection = Connection::open(file)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let ok: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if ok != "ok" {
        return Err(DeskError::db(format!("integrity check: {ok}")));
    }
    connection.execute_batch(SCHEMA)?;
    migrate(&connection)?;
    Ok(connection)
}

/// Columns added after the first release. CREATE TABLE IF NOT EXISTS never
/// changes an existing table, so each addition is checked and applied here.
fn migrate(conn: &Connection) -> Result<()> {
    let has_batch = conn.prepare("SELECT 1 FROM pragma_table_info('ops') WHERE name = 'batch'")?.exists([])?;
    if !has_batch {
        conn.execute_batch("ALTER TABLE ops ADD COLUMN batch TEXT; CREATE INDEX IF NOT EXISTS ops_batch ON ops(batch);")?;
        log::info!("database: added ops.batch");
    }
    Ok(())
}

/// An in-memory database for tests.
#[cfg(test)]
pub fn open_memory() -> Result<Connection> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(SCHEMA)?;
    migrate(&connection)?;
    Ok(connection)
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query([key])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema_and_round_trips_settings() {
        let conn = open_memory().unwrap();
        assert_eq!(get_setting(&conn, "tour_done").unwrap(), None);
        set_setting(&conn, "tour_done", "1").unwrap();
        set_setting(&conn, "tour_done", "2").unwrap();
        assert_eq!(get_setting(&conn, "tour_done").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn an_older_database_gains_the_batch_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE ops (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, src TEXT NOT NULL, dst TEXT, status TEXT NOT NULL, message TEXT, automatic INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL); INSERT INTO ops(kind, src, status, created_at) VALUES('move', 'a', 'done', 1);").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let batch: Option<String> = conn.query_row("SELECT batch FROM ops", [], |r| r.get(0)).unwrap();
        assert_eq!(batch, None, "old rows keep working");
    }

    #[test]
    fn damaged_database_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("desk.sqlite"), b"this is not a database file at all, just junk bytes").unwrap();
        let opened = open(dir.path()).unwrap();
        assert!(opened.recovered);
        let damaged = std::fs::read_dir(dir.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("damaged")).count();
        assert_eq!(damaged, 1);
    }
}
