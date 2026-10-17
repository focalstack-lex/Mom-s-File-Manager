//! The To do list (docs/superpowers/specs/2026-10-14-todo-list-design.md):
//! tasks with an optional due day, repeats, attached files and one morning
//! reminder. Tasks live in their own file, tasks.sqlite, attached to the main
//! connection as schema `todo`, so a rebuild of the index never takes them
//! along; a daily snapshot brings them back if that file is ever damaged.

use crate::commands::DateRange;
use crate::db;
use crate::error::{ErrorCode, Result, ToteError, FILE_GONE};
use crate::paths::{self, Roots};
use crate::sorter;
use chrono::{Days, Months, NaiveDate, NaiveDateTime, Timelike};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = "tasks.sqlite";
const BACKUP: &str = "tasks.backup.sqlite";
/// The copy before the latest one, so one day's snapshot never costs the list.
const PREVIOUS_BACKUP: &str = "tasks.backup.previous.sqlite";
const SNAPSHOT_KEY: &str = "tasks_snapshot_on";
/// The last day the morning reminder was shown (settings table).
pub const REMINDED_KEY: &str = "reminded_on";
const DONE_KEEP_SECS: i64 = 30 * 86_400;
const DELETED_KEEP_SECS: i64 = 7 * 86_400;
const TITLE_MAX: usize = 200;
const REMIND_FROM_HOUR: u32 = 7;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS todo.tasks (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    due TEXT,
    repeat TEXT NOT NULL DEFAULT 'never',
    done_at INTEGER,
    deleted_at INTEGER,
    repeated_from INTEGER,
    edited_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS todo.task_files (
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, path)
);
CREATE INDEX IF NOT EXISTS todo.task_files_path ON task_files(path);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    Never,
    Week,
    Month,
    Quarter,
}

impl Repeat {
    fn as_str(self) -> &'static str {
        match self {
            Repeat::Never => "never",
            Repeat::Week => "week",
            Repeat::Month => "month",
            Repeat::Quarter => "quarter",
        }
    }

    fn from_db(text: &str) -> Self {
        match text {
            "week" => Repeat::Week,
            "month" => Repeat::Month,
            "quarter" => Repeat::Quarter,
            _ => Repeat::Never,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFile {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub kind: String,
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub due: Option<String>,
    pub repeat: Repeat,
    pub done_at: Option<i64>,
    pub created_at: i64,
    pub files: Vec<TaskFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completed {
    pub task: Task,
    pub next: Option<Task>,
    pub no_next_quarter: bool,
}

/// What happened to a damaged tasks.sqlite on this start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Restored {
    Snapshot,
    Empty,
}

// ---------------------------------------------------------------------------
// Dates and words: pure, with "today" passed in.

/// "YYYY-MM-DD" as a calendar day, the form the quarters already use.
pub fn parse_day(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

fn day_text(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// Quarters with two readable dates in order, earliest first.
pub fn usable_quarters(quarters: &[DateRange]) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out: Vec<(NaiveDate, NaiveDate)> = quarters
        .iter()
        .filter_map(|q| Some((parse_day(&q.start)?, parse_day(&q.end)?)))
        .filter(|(start, end)| start <= end)
        .collect();
    out.sort();
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Due(NaiveDate),
    /// A quarterly task in the last quarter Settings knows about.
    NoNextQuarter,
}

/// When the next copy of a repeating task is due.
pub fn next_due(due: NaiveDate, repeat: Repeat, quarters: &[DateRange]) -> Option<Next> {
    match repeat {
        Repeat::Never => None,
        Repeat::Week => due.checked_add_days(Days::new(7)).map(Next::Due),
        // chrono uses the month's last day when the day does not exist (31 January to 28 February).
        Repeat::Month => due.checked_add_months(Months::new(1)).map(Next::Due),
        Repeat::Quarter => Some(next_quarter_due(due, &usable_quarters(quarters))),
    }
}

/// Same number of days before the end of the next quarter as this one was,
/// never before that quarter starts. A day outside every quarter goes to the
/// end of the next quarter that starts after it.
fn next_quarter_due(due: NaiveDate, quarters: &[(NaiveDate, NaiveDate)]) -> Next {
    // Sorted by start, so the first match is the soonest.
    let starting_after = |day: NaiveDate| quarters.iter().find(|(start, _)| *start > day).copied();
    match quarters.iter().find(|(start, end)| *start <= due && due <= *end) {
        Some(&(_, end)) => match starting_after(end) {
            Some((start, next_end)) => {
                let before_end = end.signed_duration_since(due).num_days();
                let day = u64::try_from(before_end).ok().and_then(|days| next_end.checked_sub_days(Days::new(days))).unwrap_or(next_end);
                Next::Due(day.max(start))
            }
            None => Next::NoNextQuarter,
        },
        None => match starting_after(due) {
            Some((_, next_end)) => Next::Due(next_end),
            None => Next::NoNextQuarter,
        },
    }
}

/// A task title: one line, 1 to 200 characters.
pub fn clean_title(title: &str) -> Result<String> {
    let one_line: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let cleaned = one_line.trim();
    if cleaned.is_empty() {
        return Err(ToteError::validation("Type what needs doing first."));
    }
    if cleaned.chars().count() > TITLE_MAX {
        return Err(ToteError::validation("That is too long for one task. Keep it under 200 letters."));
    }
    Ok(cleaned.to_string())
}

/// An optional due day; empty means no date.
pub fn clean_due(due: Option<&str>) -> Result<Option<NaiveDate>> {
    match due.map(str::trim).filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => parse_day(text).map(Some).ok_or_else(|| ToteError::validation("That date could not be read.")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub title: String,
    pub body: String,
    pub due_today: usize,
    pub overdue: usize,
}

/// The morning reminder, if one is due now: from 07:00, at most once a day,
/// only with the notifications switch on and something due today or overdue.
/// `open` is every open task that has a due day.
pub fn reminder(now: NaiveDateTime, last: Option<NaiveDate>, open: &[(String, NaiveDate)], enabled: bool) -> Option<Reminder> {
    let today = now.date();
    if !enabled || now.hour() < REMIND_FROM_HOUR || last == Some(today) {
        return None;
    }
    let due_today: Vec<&str> = open.iter().filter(|(_, due)| *due == today).map(|(title, _)| title.as_str()).collect();
    let overdue = open.iter().filter(|(_, due)| *due < today).count();
    if due_today.is_empty() && overdue == 0 {
        return None;
    }
    let body = if due_today.is_empty() {
        if overdue == 1 {
            "1 task is overdue.".to_string()
        } else {
            format!("{overdue} tasks are overdue.")
        }
    } else {
        let shown = due_today.iter().take(3).copied().collect::<Vec<_>>().join(", ");
        let more = due_today.len().saturating_sub(3);
        let mut text = if more > 0 { format!("{shown} and {more} more.") } else { format!("{shown}.") };
        if overdue > 0 {
            text.push_str(&format!(" Also {overdue} overdue."));
        }
        text
    };
    Some(Reminder { title: "To do today".into(), body, due_today: due_today.len(), overdue })
}

// ---------------------------------------------------------------------------
// The store.

fn not_found() -> ToteError {
    // Validation, not NotFound: the renderer words NotFound as a missing file.
    ToteError::validation("That task is no longer on the list.")
}

/// Opens tasks.sqlite beside the main database and attaches it as `todo`.
/// A file that cannot be read is set aside (never deleted) and the daily
/// snapshot takes its place; without a healthy snapshot the list starts
/// empty. A file that is missing while a healthy snapshot exists is damage
/// too (a cleaner, a failed restore): starting empty would let the next
/// snapshot replace the good copy.
pub fn open_store(conn: &Connection, data_dir: &Path) -> Result<Option<Restored>> {
    let file = data_dir.join(FILE);
    let mut restored = None;
    if file.exists() && !healthy(&file) {
        set_aside(data_dir, db::now())?;
        match usable_backup(data_dir) {
            Some((name, backup)) => {
                std::fs::copy(&backup, &file)?;
                log::warn!("{FILE} could not be read; restored it from {name}");
                restored = Some(Restored::Snapshot);
            }
            None => {
                log::warn!("{FILE} could not be read and no healthy snapshot exists; the list starts empty");
                restored = Some(Restored::Empty);
            }
        }
    } else if !file.exists() {
        if let Some((name, backup)) = usable_backup(data_dir) {
            // A -wal left beside the missing file would be replayed onto
            // the copy: SQLite checks a WAL against its own header, not
            // against the database, so it goes aside first.
            set_aside(data_dir, db::now())?;
            std::fs::copy(&backup, &file)?;
            log::warn!("{FILE} was missing; restored it from {name}");
            restored = Some(Restored::Snapshot);
        }
    }
    conn.execute("ATTACH DATABASE ?1 AS todo", [paths::display(&file)])?;
    conn.pragma_update(Some("todo"), "journal_mode", "WAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(restored)
}

/// The newest snapshot that still reads, with its file name: the latest
/// copy, else the one before it.
fn usable_backup(data_dir: &Path) -> Option<(&'static str, PathBuf)> {
    [BACKUP, PREVIOUS_BACKUP].into_iter().map(|name| (name, data_dir.join(name))).find(|(_, copy)| copy.exists() && healthy(copy))
}

/// Opened read-write, like the main database, so a WAL left by a crash is
/// recovered instead of being mistaken for damage.
fn healthy(file: &Path) -> bool {
    let Ok(conn) = Connection::open(file) else { return false };
    conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)).map(|ok| ok == "ok").unwrap_or(false)
}

fn set_aside(data_dir: &Path, stamp: i64) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let from = data_dir.join(format!("{FILE}{suffix}"));
        if from.exists() {
            std::fs::rename(&from, data_dir.join(format!("tasks.damaged-{stamp}.sqlite{suffix}")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn attach_memory(conn: &Connection) -> Result<()> {
    conn.execute_batch("ATTACH DATABASE ':memory:' AS todo")?;
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Copies the list to tasks.backup.sqlite at most once a day, through a
/// temporary file so a failed copy never replaces a good one. The copy it
/// replaces is kept as tasks.backup.previous.sqlite, and a copy that holds
/// tasks (either generation) is never replaced by a list that holds none:
/// an empty list is more likely a loss than a choice (audit T-03).
pub fn snapshot(conn: &Connection, data_dir: &Path, today: NaiveDate) -> Result<bool> {
    let stamp = day_text(today);
    if db::get_setting(conn, SNAPSHOT_KEY)?.as_deref() == Some(stamp.as_str()) {
        return Ok(false);
    }
    let backup = data_dir.join(BACKUP);
    let previous = data_dir.join(PREVIOUS_BACKUP);
    if live_count(conn)? == 0 {
        if let Some(name) = [BACKUP, PREVIOUS_BACKUP].into_iter().find(|name| {
            let copy = data_dir.join(name);
            copy.exists() && backup_count(&copy) > 0
        }) {
            log::warn!("tasks snapshot skipped: the list is empty while {name} holds tasks");
            return Ok(false);
        }
    }
    let temp = data_dir.join("tasks.backup.tmp");
    if temp.exists() {
        std::fs::remove_file(&temp)?;
    }
    conn.execute("VACUUM todo INTO ?1", [paths::display(&temp)])?;
    // The latest copy keeps its name until the new one takes it, so a
    // failure between the two steps never leaves the folder without a
    // backup; a copy that cannot be read is not kept over the one before it.
    if backup.exists() && healthy(&backup) {
        std::fs::copy(&backup, &previous)?;
    }
    std::fs::rename(&temp, &backup)?;
    db::set_setting(conn, SNAPSHOT_KEY, &stamp)?;
    Ok(true)
}

/// Tasks on the list, open or done, not deleted.
fn live_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM todo.tasks WHERE deleted_at IS NULL", [], |r| r.get(0))?)
}

/// Tasks in a backup file; 0 when it cannot be read, so it is replaced.
fn backup_count(file: &Path) -> i64 {
    let Ok(conn) = Connection::open(file) else { return 0 };
    conn.query_row("SELECT count(*) FROM tasks WHERE deleted_at IS NULL", [], |r| r.get(0)).unwrap_or(0)
}

/// Removes tasks deleted over a week ago and tasks done over 30 days ago.
pub fn purge(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM todo.tasks WHERE (deleted_at IS NOT NULL AND deleted_at < ?1) OR (done_at IS NOT NULL AND done_at < ?2)",
        params![now - DELETED_KEEP_SECS, now - DONE_KEEP_SECS],
    )?)
}

const COLUMNS: &str = "id, title, due, repeat, done_at, created_at";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let repeat: String = row.get(3)?;
    Ok(Task { id: row.get(0)?, title: row.get(1)?, due: row.get(2)?, repeat: Repeat::from_db(&repeat), done_at: row.get(4)?, created_at: row.get(5)?, files: Vec::new() })
}

fn files_of(conn: &Connection, id: i64) -> Result<Vec<TaskFile>> {
    let mut stmt = conn.prepare("SELECT path FROM todo.task_files WHERE task_id = ?1 ORDER BY added_at, path")?;
    let paths = stmt.query_map([id], |row| row.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(paths
        .into_iter()
        .map(|path| {
            let file = Path::new(&path);
            let ext = sorter::extension(file);
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let kind = sorter::classify(&ext).as_str().to_string();
            let missing = !file.is_file();
            TaskFile { path, name, ext, kind, missing }
        })
        .collect())
}

pub fn get(conn: &Connection, id: i64) -> Result<Task> {
    let task = conn
        .query_row(&format!("SELECT {COLUMNS} FROM todo.tasks WHERE id = ?1 AND deleted_at IS NULL"), [id], row_to_task)
        .optional()?
        .ok_or_else(not_found)?;
    Ok(Task { files: files_of(conn, id)?, ..task })
}

/// Open tasks (dated first, by day, then oldest first) and tasks done in the last 30 days.
pub fn list(conn: &Connection, now: i64) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM todo.tasks WHERE deleted_at IS NULL AND (done_at IS NULL OR done_at >= ?1) ORDER BY done_at IS NOT NULL, due IS NULL, due, created_at, id"
    ))?;
    let tasks = stmt.query_map([now - DONE_KEEP_SECS], row_to_task)?.collect::<std::result::Result<Vec<_>, _>>()?;
    tasks.into_iter().map(|task| Ok(Task { files: files_of(conn, task.id)?, ..task })).collect()
}

/// A write that fails says not saved; `From<rusqlite::Error>` assumes a read.
fn write(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
    conn.execute(sql, params).map_err(ToteError::db_write)
}

pub fn add(conn: &Connection, title: &str, due: Option<&str>, now: i64) -> Result<Task> {
    let title = clean_title(title)?;
    let due = clean_due(due)?;
    write(conn, "INSERT INTO todo.tasks(title, due, repeat, created_at) VALUES(?1, ?2, 'never', ?3)", params![title, due.map(day_text), now])?;
    get(conn, conn.last_insert_rowid())
}

fn check_repeat(repeat: Repeat, due: Option<NaiveDate>, quarters: &[DateRange]) -> Result<()> {
    if repeat == Repeat::Never {
        return Ok(());
    }
    if due.is_none() {
        return Err(ToteError::validation("Pick a due date first. A task needs one to repeat."));
    }
    if repeat == Repeat::Quarter && usable_quarters(quarters).is_empty() {
        return Err(ToteError::validation("Add the quarter dates in Settings first."));
    }
    Ok(())
}

pub fn update(conn: &Connection, id: i64, title: &str, due: Option<&str>, repeat: Repeat, quarters: &[DateRange], now: i64) -> Result<Task> {
    let title = clean_title(title)?;
    let due = clean_due(due)?;
    check_repeat(repeat, due, quarters)?;
    let changed = write(
        conn,
        "UPDATE todo.tasks SET title = ?2, due = ?3, repeat = ?4, edited_at = ?5 WHERE id = ?1 AND deleted_at IS NULL",
        params![id, title, due.map(day_text), repeat.as_str(), now],
    )?;
    if changed == 0 {
        return Err(not_found());
    }
    get(conn, id)
}

/// Ticks a task off. A repeating one gets its next copy in the same
/// transaction, so the list never shows one without the other.
pub fn complete(conn: &Connection, id: i64, quarters: &[DateRange], now: i64) -> Result<Completed> {
    let tx = conn.unchecked_transaction().map_err(ToteError::db_write)?;
    let task = get(&tx, id)?;
    if task.done_at.is_some() {
        return Ok(Completed { task, next: None, no_next_quarter: false });
    }
    write(&tx, "UPDATE todo.tasks SET done_at = ?2 WHERE id = ?1", params![id, now])?;
    let mut next_id = None;
    let mut no_next_quarter = false;
    if let Some(due) = task.due.as_deref().and_then(parse_day) {
        match next_due(due, task.repeat, quarters) {
            Some(Next::Due(day)) => {
                write(
                    &tx,
                    "INSERT INTO todo.tasks(title, due, repeat, repeated_from, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
                    params![task.title, day_text(day), task.repeat.as_str(), id, now],
                )?;
                next_id = Some(tx.last_insert_rowid());
            }
            Some(Next::NoNextQuarter) => no_next_quarter = true,
            None => {}
        }
    }
    tx.commit().map_err(ToteError::db_write)?;
    Ok(Completed { task: get(conn, id)?, next: next_id.map(|n| get(conn, n)).transpose()?, no_next_quarter })
}

/// Undo of a tick: back on the list, and the copy that tick made goes, but
/// only while she has not touched it (edited, done or given files).
pub fn reopen(conn: &Connection, id: i64) -> Result<()> {
    let tx = conn.unchecked_transaction().map_err(ToteError::db_write)?;
    if write(&tx, "UPDATE todo.tasks SET done_at = NULL WHERE id = ?1 AND deleted_at IS NULL", [id])? == 0 {
        return Err(not_found());
    }
    write(
        &tx,
        "DELETE FROM todo.tasks WHERE repeated_from = ?1 AND edited_at IS NULL AND done_at IS NULL AND deleted_at IS NULL \
         AND NOT EXISTS (SELECT 1 FROM todo.task_files f WHERE f.task_id = todo.tasks.id)",
        [id],
    )?;
    tx.commit().map_err(ToteError::db_write)?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64, now: i64) -> Result<()> {
    if write(conn, "UPDATE todo.tasks SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, now])? == 0 {
        return Err(not_found());
    }
    Ok(())
}

pub fn restore(conn: &Connection, id: i64) -> Result<()> {
    if write(conn, "UPDATE todo.tasks SET deleted_at = NULL WHERE id = ?1", [id])? == 0 {
        return Err(not_found());
    }
    Ok(())
}

/// Attaches a file inside her folders (the same boundary as Open).
pub fn attach_file(conn: &Connection, roots: &Roots, id: i64, path: &Path, now: i64) -> Result<Task> {
    let file = roots.confine(path)?;
    if !file.is_file() {
        return Err(ToteError::new(ErrorCode::NotFound, FILE_GONE).with_path(paths::display(&file)));
    }
    get(conn, id)?;
    write(conn, "INSERT OR IGNORE INTO todo.task_files(task_id, path, added_at) VALUES(?1, ?2, ?3)", params![id, paths::display(&file), now])?;
    get(conn, id)
}

pub fn detach_file(conn: &Connection, id: i64, path: &str) -> Result<Task> {
    write(conn, "DELETE FROM todo.task_files WHERE task_id = ?1 AND path = ?2", params![id, path])?;
    get(conn, id)
}

/// Open tasks that have a due day, for the morning reminder.
pub fn open_with_due(conn: &Connection) -> Result<Vec<(String, NaiveDate)>> {
    let mut stmt = conn.prepare("SELECT title, due FROM todo.tasks WHERE done_at IS NULL AND deleted_at IS NULL AND due IS NOT NULL ORDER BY due, created_at, id")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().filter_map(|(title, due)| Some((title, parse_day(&due)?))).collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_000_000;

    fn d(text: &str) -> NaiveDate {
        parse_day(text).unwrap()
    }

    fn q(name: &str, start: &str, end: &str) -> DateRange {
        DateRange { name: name.into(), start: start.into(), end: end.into() }
    }

    fn quarters() -> Vec<DateRange> {
        vec![
            q("Quarter 2", "2026-08-24", "2026-10-31"),
            q("Quarter 1", "2026-06-08", "2026-08-15"),
            q("Quarter 3", "2026-11-03", "2027-01-23"),
            q("Empty", "", ""),
        ]
    }

    fn memory() -> Connection {
        db::open_memory().unwrap()
    }

    #[test]
    fn weekly_and_monthly_repeats() {
        assert_eq!(next_due(d("2026-10-14"), Repeat::Week, &[]), Some(Next::Due(d("2026-10-21"))));
        assert_eq!(next_due(d("2026-12-30"), Repeat::Week, &[]), Some(Next::Due(d("2027-01-06"))));
        assert_eq!(next_due(d("2026-10-14"), Repeat::Month, &[]), Some(Next::Due(d("2026-11-14"))));
        assert_eq!(next_due(d("2026-01-31"), Repeat::Month, &[]), Some(Next::Due(d("2026-02-28"))));
        assert_eq!(next_due(d("2028-01-31"), Repeat::Month, &[]), Some(Next::Due(d("2028-02-29"))));
        assert_eq!(next_due(d("2026-12-15"), Repeat::Month, &[]), Some(Next::Due(d("2027-01-15"))));
        assert_eq!(next_due(d("2026-10-14"), Repeat::Never, &[]), None);
    }

    #[test]
    fn quarterly_keeps_the_distance_to_the_quarter_end() {
        let qs = quarters();
        // Five days before Quarter 1 ends: five days before Quarter 2 ends.
        assert_eq!(next_due(d("2026-08-10"), Repeat::Quarter, &qs), Some(Next::Due(d("2026-10-26"))));
        // Between quarters: the end of the next quarter.
        assert_eq!(next_due(d("2026-08-20"), Repeat::Quarter, &qs), Some(Next::Due(d("2026-10-31"))));
        // In the last quarter there is nothing to repeat into.
        assert_eq!(next_due(d("2026-12-01"), Repeat::Quarter, &qs), Some(Next::NoNextQuarter));
        // A short next quarter never puts the date before its start.
        let short = vec![q("A", "2026-06-08", "2026-08-15"), q("B", "2026-08-20", "2026-09-01")];
        assert_eq!(next_due(d("2026-06-08"), Repeat::Quarter, &short), Some(Next::Due(d("2026-08-20"))));
        assert_eq!(usable_quarters(&qs).len(), 3);
    }

    #[test]
    fn titles_and_dates_are_checked() {
        assert_eq!(clean_title("  Submit SF2\n ").unwrap(), "Submit SF2");
        assert!(clean_title("   ").is_err());
        assert!(clean_title(&"x".repeat(201)).is_err());
        assert_eq!(clean_title(&"x".repeat(200)).unwrap().len(), 200);
        assert_eq!(clean_due(None).unwrap(), None);
        assert_eq!(clean_due(Some(" ")).unwrap(), None);
        assert_eq!(clean_due(Some("2026-10-20")).unwrap(), Some(d("2026-10-20")));
        assert!(clean_due(Some("2026-02-30")).is_err());
        assert!(clean_due(Some("20 October")).is_err());
    }

    fn at(day: &str, hour: u32) -> NaiveDateTime {
        d(day).and_hms_opt(hour, 0, 0).unwrap()
    }

    #[test]
    fn the_morning_reminder() {
        let open = vec![
            ("Submit SF2".to_string(), d("2026-10-14")),
            ("Print quiz".to_string(), d("2026-10-14")),
            ("Check essays".to_string(), d("2026-10-14")),
            ("Call parents".to_string(), d("2026-10-14")),
            ("Bulletin board".to_string(), d("2026-10-14")),
            ("Grades".to_string(), d("2026-10-10")),
            ("Next week".to_string(), d("2026-10-20")),
        ];
        assert_eq!(reminder(at("2026-10-14", 6), None, &open, true), None, "not before 07:00");
        assert_eq!(reminder(at("2026-10-14", 9), None, &open, false), None, "switch off");
        assert_eq!(reminder(at("2026-10-14", 9), Some(d("2026-10-14")), &open, true), None, "once a day");
        let shown = reminder(at("2026-10-14", 14), Some(d("2026-10-13")), &open, true).unwrap();
        assert_eq!(shown.title, "To do today");
        assert_eq!(shown.body, "Submit SF2, Print quiz, Check essays and 2 more. Also 1 overdue.");
        assert_eq!((shown.due_today, shown.overdue), (5, 1));
        let one = vec![("Submit SF2".to_string(), d("2026-10-14"))];
        assert_eq!(reminder(at("2026-10-14", 7), None, &one, true).unwrap().body, "Submit SF2.");
        let late = vec![("Grades".to_string(), d("2026-10-10")), ("Forms".to_string(), d("2026-10-11"))];
        assert_eq!(reminder(at("2026-10-14", 8), None, &late, true).unwrap().body, "2 tasks are overdue.");
        assert_eq!(reminder(at("2026-10-14", 8), None, &late[..1], true).unwrap().body, "1 task is overdue.");
        let future = vec![("Next week".to_string(), d("2026-10-20"))];
        assert_eq!(reminder(at("2026-10-14", 8), None, &future, true), None, "nothing due");
    }

    /// A task add, tick, delete or detach that fails says not saved, never
    /// not read (audit T-18); reads still answer.
    #[test]
    fn a_failed_task_write_says_not_saved() {
        let conn = memory();
        let task = add(&conn, "Print the quiz", Some("2026-10-20"), NOW).unwrap();
        conn.pragma_update(None, "query_only", "ON").unwrap();
        let not_saved = (ErrorCode::Db, crate::error::DB_WRITE.to_string());
        let words = |error: ToteError| (error.code, error.message);
        assert_eq!(words(add(&conn, "Submit SF2", None, NOW).unwrap_err()), not_saved, "add");
        assert_eq!(words(complete(&conn, task.id, &[], NOW).unwrap_err()), not_saved, "complete");
        assert_eq!(words(delete(&conn, task.id, NOW).unwrap_err()), not_saved, "delete");
        assert_eq!(words(detach_file(&conn, task.id, "x").unwrap_err()), not_saved, "detach");
        assert_eq!(list(&conn, NOW).unwrap().len(), 1, "reads still work");
        conn.pragma_update(None, "query_only", "OFF").unwrap();
        assert!(delete(&conn, task.id, NOW).is_ok());
    }

    #[test]
    fn add_list_update_and_validation() {
        let conn = memory();
        let a = add(&conn, "Print the quiz", None, NOW).unwrap();
        let b = add(&conn, "Submit SF2", Some("2026-10-20"), NOW + 1).unwrap();
        assert_eq!(b.due.as_deref(), Some("2026-10-20"));
        assert_eq!(b.repeat, Repeat::Never);
        let titles: Vec<String> = list(&conn, NOW).unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, ["Submit SF2", "Print the quiz"], "dated first, then undated");
        assert!(update(&conn, a.id, "Print", None, Repeat::Month, &[], NOW).is_err(), "a repeat needs a due date");
        assert!(update(&conn, b.id, "Submit SF2", Some("2026-10-20"), Repeat::Quarter, &[], NOW).is_err(), "quarter needs quarters");
        let changed = update(&conn, b.id, "Submit SF2 (Oct)", Some("2026-10-21"), Repeat::Month, &[], NOW).unwrap();
        assert_eq!((changed.title.as_str(), changed.due.as_deref(), changed.repeat), ("Submit SF2 (Oct)", Some("2026-10-21"), Repeat::Month));
        assert!(add(&conn, " ", None, NOW).is_err());
        assert!(update(&conn, 999, "x", None, Repeat::Never, &[], NOW).is_err());
    }

    #[test]
    fn ticking_a_monthly_task_makes_the_next_one_and_undo_removes_it() {
        let conn = memory();
        let task = add(&conn, "Submit SF2", Some("2026-01-31"), NOW).unwrap();
        update(&conn, task.id, "Submit SF2", Some("2026-01-31"), Repeat::Month, &[], NOW).unwrap();
        let done = complete(&conn, task.id, &[], NOW + 10).unwrap();
        assert_eq!(done.task.done_at, Some(NOW + 10));
        let next = done.next.unwrap();
        assert_eq!((next.due.as_deref(), next.repeat, next.files.len()), (Some("2026-02-28"), Repeat::Month, 0));
        assert_eq!(list(&conn, NOW + 10).unwrap().len(), 2);
        reopen(&conn, task.id).unwrap();
        let after: Vec<Task> = list(&conn, NOW + 10).unwrap();
        assert_eq!(after.len(), 1, "the untouched copy goes");
        assert_eq!(after[0].done_at, None);
    }

    #[test]
    fn undo_keeps_a_copy_she_already_changed() {
        let conn = memory();
        let task = add(&conn, "Weekly plan", Some("2026-10-14"), NOW).unwrap();
        update(&conn, task.id, "Weekly plan", Some("2026-10-14"), Repeat::Week, &[], NOW).unwrap();
        let next = complete(&conn, task.id, &[], NOW).unwrap().next.unwrap();
        update(&conn, next.id, "Weekly plan, Grade 7", Some("2026-10-21"), Repeat::Week, &[], NOW).unwrap();
        reopen(&conn, task.id).unwrap();
        assert_eq!(list(&conn, NOW).unwrap().len(), 2);
    }

    #[test]
    fn the_last_quarter_does_not_repeat() {
        let conn = memory();
        let qs = vec![q("Quarter 4", "2027-01-26", "2027-04-04")];
        let task = add(&conn, "Report cards", Some("2027-04-01"), NOW).unwrap();
        update(&conn, task.id, "Report cards", Some("2027-04-01"), Repeat::Quarter, &qs, NOW).unwrap();
        let done = complete(&conn, task.id, &qs, NOW).unwrap();
        assert!(done.next.is_none());
        assert!(done.no_next_quarter);
    }

    #[test]
    fn delete_restore_and_purge() {
        let conn = memory();
        let task = add(&conn, "Old", None, NOW).unwrap();
        delete(&conn, task.id, NOW).unwrap();
        assert!(list(&conn, NOW).unwrap().is_empty());
        restore(&conn, task.id).unwrap();
        assert_eq!(list(&conn, NOW).unwrap().len(), 1);
        delete(&conn, task.id, NOW).unwrap();
        assert_eq!(purge(&conn, NOW + 6 * 86_400).unwrap(), 0, "kept for a week");
        assert_eq!(purge(&conn, NOW + 8 * 86_400).unwrap(), 1);
        let done = add(&conn, "Done long ago", None, NOW).unwrap();
        complete(&conn, done.id, &[], NOW).unwrap();
        assert_eq!(list(&conn, NOW + 29 * 86_400).unwrap().len(), 1, "Done shows 30 days");
        assert!(list(&conn, NOW + 31 * 86_400).unwrap().is_empty());
        assert_eq!(purge(&conn, NOW + 31 * 86_400).unwrap(), 1);
    }

    #[test]
    fn attachments_follow_a_move_and_show_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        // Resolved first, as ops::move_inner resolves before index::relocate:
        // CI's temp folder sits under a short name (RUNNER~1) that resolves
        // to the long one, so raw paths would not match the stored ones.
        let base = paths::resolve(dir.path()).unwrap();
        let documents = base.join("Documents");
        std::fs::create_dir_all(documents.join("Forms")).unwrap();
        let roots = Roots::explicit(base.join("Desktop"), documents.clone(), base.join("Downloads"));
        let conn = memory();
        let from = documents.join("SF2 October.xlsx");
        std::fs::write(&from, b"x").unwrap();
        let task = add(&conn, "Submit SF2", None, NOW).unwrap();
        let with = attach_file(&conn, &roots, task.id, &from, NOW).unwrap();
        assert_eq!((with.files.len(), with.files[0].kind.as_str(), with.files[0].missing), (1, "excel", false));
        assert!(attach_file(&conn, &roots, task.id, &documents.join("nope.docx"), NOW).is_err());
        assert!(attach_file(&conn, &roots, task.id, &std::env::temp_dir().join("outside.docx"), NOW).is_err());
        let to = documents.join("Forms").join("SF2 October.xlsx");
        std::fs::rename(&from, &to).unwrap();
        crate::index::relocate(&conn, &from, &to).unwrap();
        let moved = get(&conn, task.id).unwrap();
        assert_eq!(moved.files[0].path, to.to_string_lossy());
        std::fs::remove_file(&to).unwrap();
        assert!(get(&conn, task.id).unwrap().files[0].missing);
        assert!(detach_file(&conn, task.id, &to.to_string_lossy()).unwrap().files.is_empty());
    }

    #[test]
    fn a_damaged_list_comes_back_from_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = db::open(dir.path()).unwrap().connection;
            assert_eq!(open_store(&conn, dir.path()).unwrap(), None);
            add(&conn, "Submit SF2", Some("2026-10-20"), NOW).unwrap();
            assert!(snapshot(&conn, dir.path(), d("2026-10-14")).unwrap());
            assert!(!snapshot(&conn, dir.path(), d("2026-10-14")).unwrap(), "once a day");
        }
        std::fs::write(dir.path().join(FILE), b"junk, not a database at all").unwrap();
        let conn = db::open(dir.path()).unwrap().connection;
        assert_eq!(open_store(&conn, dir.path()).unwrap(), Some(Restored::Snapshot));
        assert_eq!(list(&conn, NOW).unwrap()[0].title, "Submit SF2");
        let damaged = std::fs::read_dir(dir.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("tasks.damaged-")).count();
        assert!(damaged >= 1);
    }

    #[test]
    fn a_damaged_list_without_a_snapshot_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"junk, not a database at all").unwrap();
        let conn = db::open(dir.path()).unwrap().connection;
        assert_eq!(open_store(&conn, dir.path()).unwrap(), Some(Restored::Empty));
        assert!(list(&conn, NOW).unwrap().is_empty());
    }

    #[test]
    fn rebuilding_the_index_leaves_the_list_alone() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = db::open(dir.path()).unwrap().connection;
            open_store(&conn, dir.path()).unwrap();
            add(&conn, "Keep me", None, NOW).unwrap();
        }
        std::fs::write(dir.path().join("tote.sqlite"), b"junk, not a database at all").unwrap();
        let opened = db::open(dir.path()).unwrap();
        assert!(opened.recovered);
        assert_eq!(open_store(&opened.connection, dir.path()).unwrap(), None);
        assert_eq!(list(&opened.connection, NOW).unwrap()[0].title, "Keep me");
    }

    #[test]
    fn open_tasks_with_a_due_day_feed_the_reminder() {
        let conn = memory();
        add(&conn, "Today", Some("2026-10-14"), NOW).unwrap();
        add(&conn, "No date", None, NOW).unwrap();
        let done = add(&conn, "Done", Some("2026-10-14"), NOW).unwrap();
        complete(&conn, done.id, &[], NOW).unwrap();
        assert_eq!(open_with_due(&conn).unwrap(), vec![("Today".to_string(), d("2026-10-14"))]);
    }

    fn titles_in(file: &Path) -> Vec<String> {
        let conn = Connection::open(file).unwrap();
        let mut stmt = conn.prepare("SELECT title FROM tasks WHERE deleted_at IS NULL ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<std::result::Result<Vec<String>, _>>().unwrap()
    }

    fn backup_titles(dir: &Path) -> Vec<String> {
        titles_in(&dir.join(BACKUP))
    }

    fn live_titles(conn: &Connection) -> Vec<String> {
        list(conn, NOW).unwrap().into_iter().map(|t| t.title).collect()
    }

    fn names_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_missing_list_comes_back_from_the_snapshot_and_the_backup_keeps_its_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let stale_wal = dir.path().join("stale.wal");
        {
            let conn = db::open(dir.path()).unwrap().connection;
            open_store(&conn, dir.path()).unwrap();
            add(&conn, "Submit SF2", Some("2026-10-20"), NOW).unwrap();
            add(&conn, "Print report cards", None, NOW).unwrap();
            assert!(snapshot(&conn, dir.path(), d("2026-10-14")).unwrap());
            // A change after the snapshot sits in the WAL until the file
            // closes; a copy of that WAL stands in for the one Tote leaves
            // behind when it is killed.
            add(&conn, "Only in the WAL", None, NOW).unwrap();
            std::fs::copy(dir.path().join(format!("{FILE}-wal")), &stale_wal).unwrap();
        }
        // A cleaner takes the list and leaves SQLite's own file behind.
        std::fs::remove_file(dir.path().join(FILE)).unwrap();
        std::fs::rename(&stale_wal, dir.path().join(format!("{FILE}-wal"))).unwrap();
        let conn = db::open(dir.path()).unwrap().connection;
        assert_eq!(open_store(&conn, dir.path()).unwrap(), Some(Restored::Snapshot));
        assert!(healthy(&dir.path().join(FILE)), "the restored copy reads");
        assert_eq!(live_titles(&conn), ["Submit SF2", "Print report cards"], "the stale WAL was not replayed onto the copy");
        let names = names_in(dir.path());
        assert!(names.iter().any(|n| n.starts_with("tasks.damaged-") && n.ends_with(".sqlite-wal")), "the stale WAL was set aside: {names:?}");
        assert!(snapshot(&conn, dir.path(), d("2026-10-15")).unwrap(), "the next day's snapshot runs");
        assert_eq!(backup_titles(dir.path()), ["Submit SF2", "Print report cards"]);
    }

    #[test]
    fn an_unreadable_latest_copy_falls_back_to_the_one_before_it() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = db::open(dir.path()).unwrap().connection;
            open_store(&conn, dir.path()).unwrap();
            add(&conn, "Submit SF2", None, NOW).unwrap();
            assert!(snapshot(&conn, dir.path(), d("2026-10-14")).unwrap());
            add(&conn, "Print report cards", None, NOW).unwrap();
            assert!(snapshot(&conn, dir.path(), d("2026-10-15")).unwrap());
        }
        // The list and the latest copy are both lost; the copy before it reads.
        std::fs::write(dir.path().join(BACKUP), b"junk, not a database at all").unwrap();
        std::fs::remove_file(dir.path().join(FILE)).unwrap();
        {
            let conn = db::open(dir.path()).unwrap().connection;
            assert_eq!(open_store(&conn, dir.path()).unwrap(), Some(Restored::Snapshot), "a missing list");
            assert_eq!(live_titles(&conn), ["Submit SF2"], "the copy before the latest one");
        }
        std::fs::write(dir.path().join(FILE), b"junk, not a database at all").unwrap();
        let conn = db::open(dir.path()).unwrap().connection;
        assert_eq!(open_store(&conn, dir.path()).unwrap(), Some(Restored::Snapshot), "a damaged list");
        assert_eq!(live_titles(&conn), ["Submit SF2"]);
    }

    #[test]
    fn an_empty_list_never_replaces_the_copy_before_the_latest_either() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(dir.path()).unwrap().connection;
        open_store(&conn, dir.path()).unwrap();
        let a = add(&conn, "Submit SF2", None, NOW).unwrap();
        assert!(snapshot(&conn, dir.path(), d("2026-10-14")).unwrap());
        let b = add(&conn, "Print report cards", None, NOW).unwrap();
        assert!(snapshot(&conn, dir.path(), d("2026-10-15")).unwrap());
        // The latest copy is damaged; the one before it still holds a task.
        std::fs::write(dir.path().join(BACKUP), b"junk, not a database at all").unwrap();
        delete(&conn, a.id, NOW).unwrap();
        delete(&conn, b.id, NOW).unwrap();
        assert!(!snapshot(&conn, dir.path(), d("2026-10-16")).unwrap(), "an empty list never replaces the copy before the latest one");
        assert_eq!(titles_in(&dir.path().join(PREVIOUS_BACKUP)), ["Submit SF2"]);
        add(&conn, "Call parents", None, NOW).unwrap();
        assert!(snapshot(&conn, dir.path(), d("2026-10-16")).unwrap());
        assert_eq!(backup_titles(dir.path()), ["Call parents"]);
        assert_eq!(titles_in(&dir.path().join(PREVIOUS_BACKUP)), ["Submit SF2"], "a copy that cannot be read is not kept over the one before it");
    }

    #[test]
    fn the_snapshot_never_replaces_tasks_with_an_empty_list_and_keeps_the_previous_copy() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(dir.path()).unwrap().connection;
        open_store(&conn, dir.path()).unwrap();
        let a = add(&conn, "Submit SF2", None, NOW).unwrap();
        let b = add(&conn, "Print report cards", None, NOW).unwrap();
        assert!(snapshot(&conn, dir.path(), d("2026-10-14")).unwrap());
        delete(&conn, a.id, NOW).unwrap();
        delete(&conn, b.id, NOW).unwrap();
        assert!(!snapshot(&conn, dir.path(), d("2026-10-15")).unwrap(), "an empty list never replaces a copy that holds tasks");
        assert_eq!(backup_titles(dir.path()), ["Submit SF2", "Print report cards"]);
        assert!(!dir.path().join(PREVIOUS_BACKUP).exists());
        add(&conn, "Call parents", None, NOW).unwrap();
        assert!(snapshot(&conn, dir.path(), d("2026-10-15")).unwrap(), "with a task on the list the day's snapshot is taken");
        assert_eq!(backup_titles(dir.path()), ["Call parents"]);
        let previous = Connection::open(dir.path().join(PREVIOUS_BACKUP)).unwrap();
        let kept: i64 = previous.query_row("SELECT count(*) FROM tasks WHERE deleted_at IS NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 2, "the copy before it is kept as the previous backup");
    }
}
