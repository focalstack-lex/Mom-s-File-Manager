//! The one SQLite database: the file index with full-text search, pins,
//! templates, the operations log and settings. A damaged database is moved
//! aside and rebuilt from disk, since her files are the real source of truth.

use crate::error::{ToteError, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub struct Opened {
    pub connection: Connection,
    pub recovered: bool,
    /// Set when tote.sqlite was rebuilt on this start.
    pub rebuilt: Option<Rebuilt>,
}

/// What a rebuild could keep from the damaged file: `kept` is true when her
/// settings, pins, templates and Activity all came across (sent to the
/// renderer in AppInfo, which words the one notice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rebuilt {
    pub kept: bool,
}

/// The tables the scan cannot rebuild, copied out of a damaged file.
const KEPT_TABLES: [&str; 4] = ["settings", "pins", "templates", "ops"];

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

/// Opens (or creates) `tote.sqlite` in `data_dir`. A file that cannot be
/// used is set aside with its -wal and -shm files and a fresh one is built;
/// what lives only in the database (settings, pins, templates, Activity) is
/// copied out of the damaged file first, as far as it can still be read.
pub fn open(data_dir: &Path) -> Result<Opened> {
    std::fs::create_dir_all(data_dir)?;
    let file = data_dir.join("tote.sqlite");
    match open_at(&file) {
        Ok(connection) => Ok(Opened { connection, recovered: false, rebuilt: None }),
        Err(error) => {
            log::warn!("database at {} is unusable ({error}); rebuilding", file.display());
            let stamp = chrono::Utc::now().timestamp();
            let damaged = set_aside(data_dir, stamp)?;
            let connection = match open_at(&file) {
                Ok(connection) => connection,
                Err(error) => {
                    // SQLite clears a leftover -wal before it uses an empty
                    // file and cannot while another program holds it. Nothing
                    // is built on top of that: the damaged copy goes back and
                    // the next start runs the whole rebuild again.
                    log::warn!("database rebuild: the fresh file could not be opened ({error}); the damaged copy is put back for the next start");
                    if let Some(damaged) = &damaged {
                        put_back(data_dir, damaged, stamp);
                    }
                    return Err(error);
                }
            };
            // With no file to set aside nothing was lost.
            let kept = damaged.as_deref().is_none_or(|damaged| salvage(&connection, damaged));
            // The salvage copies the settings table whole, the marker of a
            // failed full check with it; this rebuild is the answer to it.
            if let Err(error) = connection.execute("DELETE FROM settings WHERE key = ?1", [CHECK_FAILED_KEY]) {
                log::warn!("database rebuild: the failed-check marker could not be cleared ({error}); the next start rebuilds again");
            }
            Ok(Opened { connection, recovered: true, rebuilt: Some(Rebuilt { kept }) })
        }
    }
}

/// Moves tote.sqlite and its -wal and -shm files aside under one stamp, so
/// the kept copy carries the changes SQLite had not folded in yet. Returns
/// the main copy's new path when there was one. A sidecar another program
/// holds (an antivirus, Windows Search) cannot be renamed: the -wal, which
/// carries the changes, is copied beside the kept copy instead (a copy
/// reads through a shared handle), the -shm is only logged, and both stay
/// where they are, since failing here once the main file has moved would
/// end the start.
fn set_aside(data_dir: &Path, stamp: i64) -> Result<Option<PathBuf>> {
    let mut main = None;
    for suffix in ["", "-wal", "-shm"] {
        let from = data_dir.join(format!("tote.sqlite{suffix}"));
        if !from.exists() {
            continue;
        }
        let to = data_dir.join(format!("tote.damaged-{stamp}.sqlite{suffix}"));
        match std::fs::rename(&from, &to) {
            Ok(()) if suffix.is_empty() => main = Some(to),
            Ok(()) => {}
            Err(error) if suffix.is_empty() => return Err(error.into()),
            Err(error) if suffix == "-wal" => match std::fs::copy(&from, &to) {
                Ok(_) => log::warn!("database rebuild: could not set {} aside ({error}); a copy went with the kept copy instead", from.display()),
                Err(copy_error) => log::warn!("database rebuild: could not set {} aside ({error}) or copy it ({copy_error}); the kept copy misses its last changes", from.display()),
            },
            Err(error) => log::warn!("database rebuild: could not set {} aside ({error}); left where it is", from.display()),
        }
    }
    Ok(main)
}

/// Undoes a set-aside whose fresh file could not be opened: the empty file
/// goes, the damaged copy returns under its own name, a sidecar that moved
/// comes back with it and one that was only copied loses the copy. Every
/// step that fails is logged and leaves the rest as it is; the damaged copy
/// is never removed.
fn put_back(data_dir: &Path, damaged: &Path, stamp: i64) {
    let fresh = data_dir.join("tote.sqlite");
    match std::fs::remove_file(&fresh) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            log::warn!("database rebuild: the fresh file could not be removed ({error}); the damaged copy stays at {}", damaged.display());
            return;
        }
    }
    if let Err(error) = std::fs::rename(damaged, &fresh) {
        log::warn!("database rebuild: could not put the damaged copy back ({error}); it stays at {}", damaged.display());
        return;
    }
    for suffix in ["-wal", "-shm"] {
        let aside = data_dir.join(format!("tote.damaged-{stamp}.sqlite{suffix}"));
        let home = data_dir.join(format!("tote.sqlite{suffix}"));
        if !aside.exists() {
            continue;
        }
        let result = if home.exists() { std::fs::remove_file(&aside) } else { std::fs::rename(&aside, &home) };
        if let Err(error) = result {
            log::warn!("database rebuild: could not put {} back ({error})", aside.display());
        }
    }
}

/// Copies the kept tables out of the damaged file into the fresh one, one
/// table per statement, so a table that cannot be read costs only itself.
/// Returns true when every table came across.
fn salvage(conn: &Connection, damaged: &Path) -> bool {
    if let Err(error) = conn.execute("ATTACH DATABASE ?1 AS old", [damaged.to_string_lossy().as_ref()]) {
        log::warn!("database rebuild: the damaged copy could not be read ({error})");
        return false;
    }
    let mut kept = true;
    for table in KEPT_TABLES {
        match copy_table(conn, table) {
            Ok(rows) => log::info!("database rebuild: kept {rows} rows of {table}"),
            Err(error) => {
                kept = false;
                log::warn!("database rebuild: could not keep {table} ({error})");
            }
        }
    }
    if let Err(error) = conn.execute("DETACH DATABASE old", []) {
        log::warn!("database rebuild: the damaged copy stayed attached ({error})");
    }
    kept
}

/// The columns `table` has in both files, copied in one statement; rows the
/// fresh file already holds (migration markers) are kept as they are.
fn copy_table(conn: &Connection, table: &str) -> Result<usize> {
    let old_columns = columns_of(conn, "old", table)?;
    let shared: Vec<String> = columns_of(conn, "main", table)?.into_iter().filter(|c| old_columns.contains(c)).collect();
    if shared.is_empty() {
        return Err(ToteError::db(format!("{table}: no columns in common")));
    }
    let list = shared.join(", ");
    Ok(conn.execute(&format!("INSERT OR IGNORE INTO main.{table}({list}) SELECT {list} FROM old.{table}"), [])?)
}

fn columns_of(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(names.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The app was called Desk until 2026-10-14 and kept `desk.sqlite` under the
/// `com.focalstack.desk` data folder. When Tote has no database yet and
/// that older one exists, a consistent copy is taken (VACUUM INTO also reads
/// what is still in the WAL, and works while Desk is running), so her pins,
/// templates, settings and Undo history carry over. The old folder is left
/// untouched. Returns true when a copy was made.
pub fn adopt_desk_database(data_dir: &Path, desk_dir: &Path) -> Result<bool> {
    let old = desk_dir.join("desk.sqlite");
    let new = data_dir.join("tote.sqlite");
    if new.exists() || !old.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(data_dir)?;
    let source = Connection::open_with_flags(&old, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let target = new.to_string_lossy().into_owned();
    if let Err(error) = source.execute("VACUUM INTO ?1", [&target]) {
        // A half-written copy would be opened as her database; remove it so
        // the normal start builds a fresh index instead.
        if new.exists() {
            std::fs::remove_file(&new)?;
        }
        return Err(error.into());
    }
    Ok(true)
}

fn open_at(file: &Path) -> Result<Connection> {
    let connection = Connection::open(file)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // The quick check reads every page's structure but not each index
    // against its table, so a 100 MB index no longer holds the window back
    // by the full check's second (audit T-50). The full check runs after
    // the first scan (worker::start_scan) and leaves CHECK_FAILED_KEY when
    // it finds damage; this start answers that marker with a rebuild.
    let ok: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if ok != "ok" {
        return Err(ToteError::db(format!("quick check: {ok}")));
    }
    connection.execute_batch(SCHEMA)?;
    migrate(&connection)?;
    if let Some(at) = get_setting(&connection, CHECK_FAILED_KEY)? {
        return Err(ToteError::db(format!("the full check found damage (recorded at {at})")));
    }
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
    // Since 2026-10-16 a Recycle Bin row records the file's size and the
    // moment of the delete, so Undo restores that item and no other. Since
    // 2026-10-17 an undone row records the moment of the Undo and where the
    // file landed, so the Downloads watcher still knows a put-back file when
    // the put-back was a copy or landed beside a namesake as "Name (2)".
    for (column, kind) in [("size", "INTEGER"), ("recycled_at", "INTEGER"), ("undone_at", "INTEGER"), ("undone_to", "TEXT")] {
        let present = conn.prepare("SELECT 1 FROM pragma_table_info('ops') WHERE name = ?1")?.exists([column])?;
        if !present {
            conn.execute_batch(&format!("ALTER TABLE ops ADD COLUMN {column} {kind}"))?;
            log::info!("database: added ops.{column}");
        }
    }
    // Since 2026-10-16 a name with an apostrophe is indexed with and without
    // it (index::indexed_name), so "teachers day" finds "Teacher's Day". Rows
    // written before that are rewritten once; the marker keeps it to once.
    if get_setting(conn, "fts_name_forms")?.is_none() {
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn.prepare("SELECT id, name FROM files WHERE name LIKE '%''%' OR name LIKE '%\u{2019}%' OR name LIKE '%\u{2018}%'")?;
            let found = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            found.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (id, name) in &rows {
            conn.execute("UPDATE files_fts SET name = ?2 WHERE rowid = ?1", rusqlite::params![id, crate::index::indexed_name(name)])?;
        }
        set_setting(conn, "fts_name_forms", "1")?;
        if !rows.is_empty() {
            log::info!("database: reindexed {} names with an apostrophe", rows.len());
        }
    }
    Ok(())
}

/// An in-memory database for tests.
#[cfg(test)]
pub fn open_memory() -> Result<Connection> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(SCHEMA)?;
    migrate(&connection)?;
    crate::tasks::attach_memory(&connection)?;
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

/// The setting the background check leaves when it finds damage, holding
/// the moment; `open_at` answers it with a rebuild (audit T-50).
pub const CHECK_FAILED_KEY: &str = "db_check_failed";

/// `PRAGMA integrity_check` over the whole of tote.sqlite on a connection
/// of its own, so the database lock stays free while it reads the file.
/// Ok(false) when the check lists damage; Err when it could not run.
pub fn full_check(data_dir: &Path) -> Result<bool> {
    let connection = Connection::open(data_dir.join("tote.sqlite"))?;
    let ok: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if ok != "ok" {
        log::warn!("database: integrity check: {ok}");
    }
    Ok(ok == "ok")
}

/// Records that the full check found damage; the next start rebuilds.
pub fn mark_check_failed(conn: &Connection) -> Result<()> {
    set_setting(conn, CHECK_FAILED_KEY, &now().to_string()).map_err(ToteError::as_write)
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
        let recorded: (Option<i64>, Option<i64>) = conn.query_row("SELECT size, recycled_at FROM ops", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(recorded, (None, None), "the recycle columns are added too");
        let undone: (Option<i64>, Option<String>) = conn.query_row("SELECT undone_at, undone_to FROM ops", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(undone, (None, None), "and the Undo columns");
    }

    #[test]
    fn an_older_database_reindexes_names_with_an_apostrophe_once() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO files(id, path, parent, name, ext, kind, size, modified, group_key) VALUES(1, 'C:\\a\\Teacher''s Day.docx', 'C:\\a', 'Teacher''s Day.docx', 'docx', 'word', 1, 1, 'teacher s day|docx'), (2, 'C:\\a\\Book1.xlsx', 'C:\\a', 'Book1.xlsx', 'xlsx', 'excel', 1, 1, 'book1|xlsx');
             INSERT INTO files_fts(rowid, name, content) VALUES(1, 'Teacher''s Day.docx', ''), (2, 'Book1.xlsx', '');",
        )
        .unwrap();
        migrate(&conn).unwrap();
        let fts_name = |id: i64| -> String { conn.query_row("SELECT name FROM files_fts WHERE rowid = ?1", [id], |r| r.get(0)).unwrap() };
        assert_eq!(fts_name(1), "Teacher's Day.docx Teachers Day.docx");
        assert_eq!(fts_name(2), "Book1.xlsx", "a name without an apostrophe is left alone");
        assert_eq!(get_setting(&conn, "fts_name_forms").unwrap().as_deref(), Some("1"));
        conn.execute("UPDATE files_fts SET name = 'changed' WHERE rowid = 1", []).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(fts_name(1), "changed", "the rewrite runs once");
    }

    #[test]
    fn a_desk_database_carries_over_once() {
        let desk = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let old = open(desk.path()).unwrap().connection;
        set_setting(&old, "tour_done", "1").unwrap();
        drop(old);
        // Desk's own file name; open() now writes tote.sqlite.
        std::fs::rename(desk.path().join("tote.sqlite"), desk.path().join("desk.sqlite")).unwrap();
        assert!(adopt_desk_database(data.path(), desk.path()).unwrap());
        let opened = open(data.path()).unwrap();
        assert_eq!(get_setting(&opened.connection, "tour_done").unwrap().as_deref(), Some("1"));
        assert!(!adopt_desk_database(data.path(), desk.path()).unwrap(), "never overwrites Tote's own database");
        assert!(desk.path().join("desk.sqlite").exists(), "the old folder is left alone");
    }

    #[test]
    fn nothing_to_adopt_without_desk() {
        let desk = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        assert!(!adopt_desk_database(data.path(), desk.path()).unwrap());
        assert!(!data.path().join("tote.sqlite").exists());
    }

    #[test]
    fn damaged_database_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tote.sqlite"), b"this is not a database file at all, just junk bytes").unwrap();
        let opened = open(dir.path()).unwrap();
        assert!(opened.recovered);
        assert_eq!(opened.rebuilt, Some(Rebuilt { kept: false }), "junk holds nothing to keep");
        let damaged = std::fs::read_dir(dir.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("damaged")).count();
        assert_eq!(damaged, 1);
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// Her things in a database, then one page of a `files` index
    /// overwritten: the integrity check fails while the small tables read.
    fn seed_then_damage_an_index_page(dir: &Path) {
        {
            let conn = open(dir).unwrap().connection;
            set_setting(&conn, "settings", "{\"startAtLogin\":false}").unwrap();
            conn.execute("INSERT INTO pins(path, created_at) VALUES('C:\\a\\Quiz.docx', 1)", []).unwrap();
            conn.execute("INSERT INTO templates(path, name, created_at) VALUES('C:\\a\\SF9.docx', 'SF9', 1)", []).unwrap();
            conn.execute("INSERT INTO ops(kind, src, dst, status, created_at) VALUES('move', 'C:\\a\\x.docx', 'C:\\b\\x.docx', 'done', 1)", []).unwrap();
            conn.execute("INSERT INTO files(path, parent, name, ext, kind, size, modified, group_key) VALUES('C:\\a\\x.docx', 'C:\\a', 'x.docx', 'docx', 'word', 1, 1, 'x|docx')", []).unwrap();
        }
        let file = dir.join("tote.sqlite");
        let (page, size): (i64, i64) = {
            let conn = Connection::open(&file).unwrap();
            let page = conn.query_row("SELECT rootpage FROM sqlite_master WHERE name = 'files_modified'", [], |r| r.get(0)).unwrap();
            let size = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
            (page, size)
        };
        let mut bytes = std::fs::read(&file).unwrap();
        let start = usize::try_from((page - 1) * size).unwrap();
        let end = usize::try_from(page * size).unwrap();
        for byte in &mut bytes[start..end] {
            *byte = 0xA7;
        }
        std::fs::write(&file, bytes).unwrap();
    }

    #[test]
    fn a_damaged_database_keeps_her_settings_pins_templates_and_activity() {
        let dir = tempfile::tempdir().unwrap();
        seed_then_damage_an_index_page(dir.path());
        let opened = open(dir.path()).unwrap();
        assert!(opened.recovered);
        assert_eq!(opened.rebuilt, Some(Rebuilt { kept: true }));
        let conn = opened.connection;
        let count = |table: &str| -> i64 { conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap() };
        assert_eq!(get_setting(&conn, "settings").unwrap().as_deref(), Some("{\"startAtLogin\":false}"));
        assert_eq!((count("pins"), count("templates"), count("ops")), (1, 1, 1));
        assert_eq!(count("files"), 0, "the index is rebuilt by the scan");
        let names = names_in(dir.path());
        assert!(names.iter().any(|n| n.starts_with("tote.damaged-") && n.ends_with(".sqlite")), "{names:?}");
    }

    #[test]
    fn damage_recovery_moves_the_wal_and_shm_aside_with_the_main_file() {
        let dir = tempfile::tempdir().unwrap();
        seed_then_damage_an_index_page(dir.path());
        // SQLite deletes a WAL it can fold into the file when it closes; one it
        // cannot touch (here read-only, as after a failed checkpoint) stays,
        // and the set-aside must carry it with the main file.
        for suffix in ["-wal", "-shm"] {
            let sidecar = dir.path().join(format!("tote.sqlite{suffix}"));
            std::fs::write(&sidecar, b"stale").unwrap();
            let mut perms = std::fs::metadata(&sidecar).unwrap().permissions();
            perms.set_readonly(true);
            std::fs::set_permissions(&sidecar, perms).unwrap();
        }
        let opened = open(dir.path()).unwrap();
        assert!(opened.recovered);
        let names = names_in(dir.path());
        let stamp = names.iter().find_map(|n| n.strip_prefix("tote.damaged-").and_then(|rest| rest.strip_suffix(".sqlite"))).map(str::to_string).unwrap();
        for suffix in ["-wal", "-shm"] {
            let moved = dir.path().join(format!("tote.damaged-{stamp}.sqlite{suffix}"));
            assert!(moved.exists(), "{suffix} follows the damaged copy: {names:?}");
            let mut perms = std::fs::metadata(&moved).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            std::fs::set_permissions(&moved, perms).unwrap();
        }
    }

    /// A sidecar another program holds without delete sharing cannot be
    /// renamed; the rebuild still goes on and keeps her things.
    #[cfg(windows)]
    #[test]
    fn a_held_sidecar_does_not_stop_the_rebuild() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        seed_then_damage_an_index_page(dir.path());
        let shm = dir.path().join("tote.sqlite-shm");
        std::fs::write(&shm, b"stale").unwrap();
        // FILE_SHARE_READ | FILE_SHARE_WRITE without FILE_SHARE_DELETE: a
        // rename is refused while this handle is open, as during a scan.
        let held = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(&shm).unwrap();
        let opened = open(dir.path()).unwrap();
        assert!(opened.recovered);
        assert_eq!(opened.rebuilt, Some(Rebuilt { kept: true }), "the damaged copy was still read");
        let names = names_in(dir.path());
        assert!(names.iter().any(|n| n.starts_with("tote.damaged-") && n.ends_with(".sqlite")), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("tote.damaged-") && n.ends_with(".sqlite-shm")), "the held sidecar stayed: {names:?}");
        drop(held);
    }

    /// The -wal is the sidecar that matters: it carries the last changes,
    /// and SQLite clears a leftover one before it uses an empty file, which
    /// it cannot while another program holds it. The start cannot go on,
    /// so the damaged copy goes back under its own name and the next start
    /// runs the whole rebuild with her things still in it.
    #[cfg(windows)]
    #[test]
    fn a_held_wal_puts_the_damaged_copy_back_for_the_next_start() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        seed_then_damage_an_index_page(dir.path());
        let wal = dir.path().join("tote.sqlite-wal");
        std::fs::write(&wal, b"stale").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(&wal).unwrap();
        let outcome = open(dir.path());
        assert!(outcome.is_err(), "the fresh file cannot be used while the -wal is held");
        drop(outcome);
        assert_eq!(names_in(dir.path()), vec!["tote.sqlite".to_string(), "tote.sqlite-wal".to_string()], "the damaged copy is back, with no stamped files left");
        drop(held);
        let opened = open(dir.path()).unwrap();
        assert_eq!(opened.rebuilt, Some(Rebuilt { kept: true }), "the next start keeps her things");
        assert_eq!(get_setting(&opened.connection, "settings").unwrap().as_deref(), Some("{\"startAtLogin\":false}"));
        let names = names_in(dir.path());
        assert_eq!(names.iter().filter(|n| n.starts_with("tote.damaged-") && n.ends_with(".sqlite")).count(), 1, "one damaged copy, from the start that went through: {names:?}");
    }

    /// A -wal that cannot be renamed is copied beside the kept copy instead,
    /// so the kept copy still carries the last changes; the held file stays.
    #[cfg(windows)]
    #[test]
    fn a_held_wal_is_copied_beside_the_kept_copy() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tote.sqlite"), b"main").unwrap();
        let wal = dir.path().join("tote.sqlite-wal");
        std::fs::write(&wal, b"stale").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(&wal).unwrap();
        let main = set_aside(dir.path(), 7).unwrap();
        assert_eq!(main, Some(dir.path().join("tote.damaged-7.sqlite")));
        assert_eq!(std::fs::read(dir.path().join("tote.damaged-7.sqlite-wal")).unwrap(), b"stale", "a copy went with the kept copy");
        assert_eq!(std::fs::read(&wal).unwrap(), b"stale", "the held file stays where it is");
        drop(held);
        put_back(dir.path(), &dir.path().join("tote.damaged-7.sqlite"), 7);
        assert_eq!(names_in(dir.path()), vec!["tote.sqlite".to_string(), "tote.sqlite-wal".to_string()], "put back leaves the copy out and the damaged file home");
        assert_eq!(std::fs::read(dir.path().join("tote.sqlite")).unwrap(), b"main");
    }

    /// The start runs only the quick check; the full check runs later on a
    /// connection of its own and leaves a marker that makes the next start
    /// rebuild, which the salvaged settings do not bring back (audit T-50).
    #[test]
    fn a_failed_full_check_marks_the_next_start_for_a_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = open(dir.path()).unwrap().connection;
            set_setting(&conn, "settings", "{\"startAtLogin\":false}").unwrap();
            conn.execute("INSERT INTO pins(path, created_at) VALUES('C:\\a\\Quiz.docx', 1)", []).unwrap();
            assert!(full_check(dir.path()).unwrap(), "a healthy file passes while the app holds it open");
            mark_check_failed(&conn).unwrap();
        }
        let opened = open(dir.path()).unwrap();
        assert!(opened.recovered, "the marker alone makes the start rebuild");
        assert_eq!(opened.rebuilt, Some(Rebuilt { kept: true }));
        let conn = opened.connection;
        assert_eq!(get_setting(&conn, "settings").unwrap().as_deref(), Some("{\"startAtLogin\":false}"));
        assert_eq!(conn.query_row("SELECT count(*) FROM pins", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(get_setting(&conn, CHECK_FAILED_KEY).unwrap(), None, "the marker does not come back with the settings");
        drop(conn);
        assert!(!open(dir.path()).unwrap().recovered, "one rebuild answers the marker");
        // What the full check is for: damage inside an index page.
        let damaged = tempfile::tempdir().unwrap();
        seed_then_damage_an_index_page(damaged.path());
        let found = full_check(damaged.path());
        assert!(matches!(found, Ok(false)), "the damage is listed: {found:?}");
    }
}
